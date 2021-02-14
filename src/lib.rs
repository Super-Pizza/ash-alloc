//! A collection of memory allocators for the Vulkan API.
use std::ffi::c_void;

use ash::version::DeviceV1_0;
use ash::vk;
#[cfg(feature = "tracing")]
use tracing::debug;

pub use error::AllocatorError;
pub use general_allocator::{
    GeneralAllocation, GeneralAllocationDescriptor, GeneralAllocator, GeneralAllocatorDescriptor,
};
pub use linear_allocator::{
    LinearAllocation, LinearAllocationDescriptor, LinearAllocator, LinearAllocatorDescriptor,
};

mod error;
mod general_allocator;
mod linear_allocator;

type Result<T> = std::result::Result<T, AllocatorError>;

#[inline]
fn align_down(offset: u64, alignment: u64) -> u64 {
    offset & !(alignment - 1u64)
}

#[inline]
fn align_up(offset: u64, alignment: u64) -> u64 {
    (offset + (alignment - 1u64)) & !(alignment - 1u64)
}

#[inline]
fn is_on_same_page(offset_lhs: u64, size_lhs: u64, offset_rhs: u64, page_size: u64) -> bool {
    if offset_lhs == 0 && size_lhs == 0 {
        return false;
    }

    let end_lhs = offset_lhs + size_lhs - 1;
    let end_page_lhs = align_down(end_lhs, page_size);
    let start_rhs = offset_rhs;
    let start_page_rhs = align_down(start_rhs, page_size);

    end_page_lhs == start_page_rhs
}

#[inline]
fn has_granularity_conflict(lhs_is_linear: bool, rhs_is_linear: bool) -> bool {
    lhs_is_linear != rhs_is_linear
}

/// Information about an allocation of all allocators.
pub trait Allocation {
    /// The `vk::DeviceMemory` of the allocation. Managed by the allocator.
    fn memory(&self) -> vk::DeviceMemory;

    /// The offset inside the `vk::DeviceMemory`.
    fn offset(&self) -> u64;

    /// The size of the allocation.
    fn size(&self) -> u64;

    /// Returns a pointer into the mapped memory if it is host visible, otherwise returns None.
    fn mapped_ptr(&self) -> Option<std::ptr::NonNull<c_void>>;

    /// Returns a valid mapped slice if the memory is host visible, otherwise it will return None.
    /// The slice already references the exact memory region of the sub allocation, so no offset needs to be applied.
    fn mapped_slice(&self) -> Option<&[u8]> {
        if let Some(ptr) = self.mapped_ptr() {
            unsafe {
                Some(std::slice::from_raw_parts(
                    ptr.as_ptr() as *const _,
                    self.size() as usize,
                ))
            }
        } else {
            None
        }
    }

    /// Returns a valid mapped mutable slice if the memory is host visible, otherwise it will return None.
    /// The slice already references the exact memory region of the sub allocation, so no offset needs to be applied.
    fn mapped_slice_mut(&mut self) -> Option<&mut [u8]> {
        if let Some(ptr) = self.mapped_ptr() {
            unsafe {
                Some(std::slice::from_raw_parts_mut(
                    ptr.as_ptr() as *mut _,
                    self.size() as usize,
                ))
            }
        } else {
            None
        }
    }
}

/// Trait to query a allocator for some information.
pub trait AllocatorInfo {
    /// Allocated memory in bytes.
    fn allocated(&self) -> u64;
    /// Reserved memory in bytes.
    fn size(&self) -> u64;
    /// Reserved memory blocks.
    fn reserved_blocks(&self) -> usize;
}

/// Type of the allocation.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum AllocationType {
    /// A allocation for a buffer.
    Buffer,
    /// An allocation for a regular image.
    OptimalImage,
    /// An allocation for a linear image.
    LinearImage,
}

impl AllocationType {
    /// Returns true if this is a "linear" type (buffers and linear images).
    pub(crate) fn is_linear(&self) -> bool {
        match self {
            AllocationType::Buffer => true,
            AllocationType::OptimalImage => false,
            AllocationType::LinearImage => true,
        }
    }
}

/// The location of the memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryLocation {
    /// Mainly used for uploading data to the GPU (DEVICE_LOCAL | HOST_VISIBLE | HOST_COHERENT).
    CpuToGpu,
    /// Used as fast access memory for the GPU (DEVICE_LOCAL).
    GpuOnly,
    /// Mainly used for downloading data from the GPU (HOST_VISIBLE | HOST_COHERENT | HOST_CACHED).
    GpuToCpu,
}

/// A reserved memory block.
#[derive(Debug)]
struct MemoryBlock {
    device_memory: vk::DeviceMemory,
    size: u64,
    mapped_ptr: *mut c_void,
}

impl MemoryBlock {
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn new(
        device: &ash::Device,
        size: u64,
        memory_type_index: usize,
        is_mappable: bool,
    ) -> Result<Self> {
        let device_memory = {
            let alloc_info = vk::MemoryAllocateInfo::builder()
                .allocation_size(size)
                .memory_type_index(memory_type_index as u32);

            let allocation_flags = vk::MemoryAllocateFlags::DEVICE_ADDRESS;
            let mut flags_info = vk::MemoryAllocateFlagsInfo::builder().flags(allocation_flags);

            let alloc_info = if cfg!(features = "vk-buffer-device-address") {
                alloc_info.push_next(&mut flags_info)
            } else {
                alloc_info
            };

            unsafe { device.allocate_memory(&alloc_info, None) }
                .map_err(|_| AllocatorError::OutOfMemory)?
        };

        let mapped_ptr = if is_mappable {
            unsafe {
                device.map_memory(
                    device_memory,
                    0,
                    vk::WHOLE_SIZE,
                    vk::MemoryMapFlags::empty(),
                )
            }
            .map_err(|_| {
                unsafe { device.free_memory(device_memory, None) };
                AllocatorError::FailedToMap
            })?
        } else {
            std::ptr::null_mut()
        };

        Ok(Self {
            device_memory,
            size,
            mapped_ptr,
        })
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn destroy(&mut self, device: &ash::Device) {
        if !self.mapped_ptr.is_null() {
            unsafe { device.unmap_memory(self.device_memory) };
        }

        unsafe { device.free_memory(self.device_memory, None) };
    }
}

fn find_memory_type_index(
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    location: MemoryLocation,
    memory_type_bits: u32,
) -> Result<usize> {
    // Prefer fast path memory when doing transfers between host and device.
    let memory_property_flags = match location {
        MemoryLocation::GpuOnly => vk::MemoryPropertyFlags::DEVICE_LOCAL,
        MemoryLocation::CpuToGpu => {
            vk::MemoryPropertyFlags::HOST_VISIBLE
                | vk::MemoryPropertyFlags::HOST_COHERENT
                | vk::MemoryPropertyFlags::DEVICE_LOCAL
        }
        MemoryLocation::GpuToCpu => {
            vk::MemoryPropertyFlags::HOST_VISIBLE
                | vk::MemoryPropertyFlags::HOST_COHERENT
                | vk::MemoryPropertyFlags::HOST_CACHED
        }
    };

    let mut memory_type_index_optional =
        query_memory_type_index(memory_properties, memory_type_bits, memory_property_flags);

    // Lose memory requirements if no fast path is found.
    if memory_type_index_optional.is_none() {
        let memory_property_flags = match location {
            MemoryLocation::GpuOnly => vk::MemoryPropertyFlags::DEVICE_LOCAL,
            MemoryLocation::CpuToGpu => {
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT
            }
            MemoryLocation::GpuToCpu => {
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT
            }
        };

        memory_type_index_optional =
            query_memory_type_index(memory_properties, memory_type_bits, memory_property_flags);
    }

    match memory_type_index_optional {
        Some(x) => Ok(x as usize),
        None => Err(AllocatorError::NoCompatibleMemoryTypeFound),
    }
}

fn query_memory_type_index(
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    memory_type_bits: u32,
    memory_property_flags: vk::MemoryPropertyFlags,
) -> Option<u32> {
    memory_properties.memory_types[..memory_properties.memory_type_count as usize]
        .iter()
        .enumerate()
        .find(|(index, memory_type)| {
            memory_type_is_compatible(*index, memory_type_bits)
                && memory_type.property_flags.contains(memory_property_flags)
        })
        .map(|(index, _)| index as u32)
}

#[inline]
fn memory_type_is_compatible(memory_type_index: usize, memory_type_bits: u32) -> bool {
    (1 << memory_type_index) & memory_type_bits != 0
}

#[cfg(feature = "tracing")]
fn debug_memory_types(
    memory_properties: vk::PhysicalDeviceMemoryProperties,
    memory_types: &[vk::MemoryType],
) {
    debug!("Memory heaps:");
    for i in 0..memory_properties.memory_heap_count as usize {
        if memory_properties.memory_heaps[i].flags == vk::MemoryHeapFlags::DEVICE_LOCAL {
            debug!(
                "HEAP[{}] device local [y] size: {} MiB",
                i,
                memory_properties.memory_heaps[i].size / (1024 * 1024)
            );
        } else {
            debug!(
                "HEAP[{}] device local [n] size: {} MiB",
                i,
                memory_properties.memory_heaps[i].size / (1024 * 1024)
            );
        }
    }
    debug!("Memory types:");
    for (i, memory_type) in memory_types.iter().enumerate() {
        debug!(
            "Memory type[{}] on HEAP[{}] property flags: {:?}",
            i, memory_type.heap_index, memory_type.property_flags
        );
    }
}
