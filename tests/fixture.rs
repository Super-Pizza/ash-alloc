use std::ffi::CStr;
use std::os::raw::c_char;

#[cfg(feature = "tracing")]
use ash::ext;
use ash::vk;
#[cfg(feature = "tracing")]
use tracing1::{debug, info};

#[cfg(feature = "tracing")]
pub fn initialize_logging() {
    use std::sync::Once;
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        use tracing_subscriber::filter::EnvFilter;

        let filter =
            EnvFilter::from_default_env().add_directive("lib::fixture=WARN".parse().unwrap());
        tracing_subscriber::fmt().with_env_filter(filter).init();
    });
}

const LAYER_KHRONOS_VALIDATION: *const c_char = c"VK_LAYER_KHRONOS_validation".as_ptr();

pub struct VulkanContext {
    // The order is important! Or else we will get an exception on drop!
    #[cfg(feature = "tracing")]
    debug_messenger: vk::DebugUtilsMessengerEXT,
    #[cfg(feature = "tracing")]
    debug_utils_ext: ext::debug_utils::Instance,
    pub logical_device: ash::Device,
    pub instance: ash::Instance,
    _entry: ash::Entry,

    pub physical_device: vk::PhysicalDevice,
    pub queue: vk::Queue,
    pub buffer_image_granularity: vk::DeviceSize,
}

impl Drop for VulkanContext {
    fn drop(&mut self) {
        unsafe {
            self.logical_device.destroy_device(None);

            #[cfg(feature = "tracing")]
            self.debug_utils_ext
                .destroy_debug_utils_messenger(self.debug_messenger, None);

            self.instance.destroy_instance(None);
        };
    }
}

impl VulkanContext {
    pub fn new(api_version: u32) -> Self {
        #[cfg(feature = "tracing")]
        initialize_logging();

        let entry = unsafe { ash::Entry::load().unwrap() };

        let engine_name = std::ffi::CString::new("erupt").unwrap();
        let app_name = std::ffi::CString::new("vk-alloc").unwrap();

        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name)
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(&engine_name)
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(api_version);

        let extensions = Self::create_instance_extensions(&entry);
        let instance_layers = Self::create_layers(&entry);
        let instance = Self::create_instance(&entry, &app_info, &extensions, &instance_layers);
        let (physical_device, logical_device, queue) = Self::request_device(&instance);

        let physical_device_properties =
            unsafe { instance.get_physical_device_properties(physical_device) };

        let buffer_image_granularity = physical_device_properties.limits.buffer_image_granularity;

        #[cfg(feature = "tracing")]
        {
            let (debug_messenger, debug_utils_ext) =
                Self::create_debug_messenger(&entry, &instance);
            Self {
                _entry: entry,
                instance,
                physical_device,
                logical_device,
                queue,
                buffer_image_granularity,
                debug_messenger,
                debug_utils_ext,
            }
        }

        #[cfg(not(feature = "tracing"))]
        {
            Self {
                _entry: entry,
                instance,
                physical_device,
                logical_device,
                queue,
                buffer_image_granularity,
            }
        }
    }

    fn create_instance_extensions(entry: &ash::Entry) -> Vec<*const std::os::raw::c_char> {
        let instance_extensions =
            unsafe { entry.enumerate_instance_extension_properties(None).unwrap() };

        let mut extensions = Vec::new();

        extensions.push(ash::ext::debug_utils::NAME.as_ptr());

        extensions.retain(|ext| {
            let extension = unsafe { CStr::from_ptr(*ext) };
            let found = instance_extensions.iter().any(|inst_ext| unsafe {
                CStr::from_ptr(inst_ext.extension_name.as_ptr()) == extension
            });
            if found {
                true
            } else {
                panic!(
                    "Unable to find instance extension: {}",
                    extension.to_string_lossy()
                );
            }
        });
        extensions
    }

    fn create_layers(entry: &ash::Entry) -> Vec<*const std::os::raw::c_char> {
        let instance_layers = unsafe { entry.enumerate_instance_layer_properties() }.unwrap();

        let mut layers = Vec::new();

        layers.push(LAYER_KHRONOS_VALIDATION);

        layers.retain(|layer| {
            let instance_layer = unsafe { CStr::from_ptr(*layer) };
            let found = instance_layers.iter().any(|inst_layer| unsafe {
                CStr::from_ptr(inst_layer.layer_name.as_ptr()) == instance_layer
            });
            if found {
                true
            } else {
                panic!("Unable to find layer: {}", instance_layer.to_string_lossy());
            }
        });
        layers
    }

    fn create_instance(
        entry: &ash::Entry,
        app_info: &vk::ApplicationInfo,
        extensions: &[*const c_char],
        layers: &[*const c_char],
    ) -> ash::Instance {
        let create_info = vk::InstanceCreateInfo::default()
            .flags(vk::InstanceCreateFlags::empty())
            .application_info(app_info)
            .enabled_layer_names(layers)
            .enabled_extension_names(extensions);

        unsafe { entry.create_instance(&create_info, None) }.unwrap()
    }

    #[cfg(feature = "tracing")]
    fn create_debug_messenger(
        entry: &ash::Entry,
        instance: &ash::Instance,
    ) -> (vk::DebugUtilsMessengerEXT, ext::debug_utils::Instance) {
        let info = vk::DebugUtilsMessengerCreateInfoEXT::default()
            .message_severity(
                vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                    | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                    | vk::DebugUtilsMessageSeverityFlagsEXT::INFO,
            )
            .message_type(
                vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                    | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                    | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE
                    | vk::DebugUtilsMessageTypeFlagsEXT::DEVICE_ADDRESS_BINDING,
            )
            .pfn_user_callback(Some(debug_utils_callback));

        let instance = ext::debug_utils::Instance::new(entry, instance);

        let debug_messenger =
            unsafe { instance.create_debug_utils_messenger(&info, None) }.unwrap();
        (debug_messenger, instance)
    }

    fn request_device(instance: &ash::Instance) -> (vk::PhysicalDevice, ash::Device, vk::Queue) {
        let physical_devices = unsafe { instance.enumerate_physical_devices().unwrap() };

        let mut chosen = None;
        for device in physical_devices {
            let properties = unsafe { instance.get_physical_device_properties(device) };

            if properties.device_type == vk::PhysicalDeviceType::DISCRETE_GPU
                || properties.device_type == vk::PhysicalDeviceType::INTEGRATED_GPU
            {
                chosen = Some((device, properties))
            }
        }

        let (physical_device, _) = chosen.unwrap();
        let (logical_device, queue) = Self::create_logical_device(instance, physical_device);

        (physical_device, logical_device, queue)
    }

    fn create_logical_device(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
    ) -> (ash::Device, vk::Queue) {
        let queue_family_properties =
            unsafe { instance.get_physical_device_queue_family_properties(physical_device) };

        let transfer_queue_family_id =
            Self::find_queue_family(vk::QueueFlags::TRANSFER, &queue_family_properties);

        let queue_infos = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(transfer_queue_family_id)
            .queue_priorities(&[1.0])];
        let logical_device = Self::create_device(instance, physical_device, &queue_infos);
        let queue = unsafe { logical_device.get_device_queue(transfer_queue_family_id, 0) };

        (logical_device, queue)
    }

    fn find_queue_family(
        target_family: vk::QueueFlags,
        queue_family_properties: &[vk::QueueFamilyProperties],
    ) -> u32 {
        let mut queue_id = None;
        for (id, family) in queue_family_properties.iter().enumerate() {
            match target_family {
                vk::QueueFlags::TRANSFER => {
                    if family.queue_count > 0
                        && family.queue_flags.contains(vk::QueueFlags::TRANSFER)
                        && queue_id.is_none()
                    {
                        queue_id = Some(id as u32);
                    }
                }
                _ => panic!("Unhandled vk::QueueFlags value"),
            }
        }

        queue_id.unwrap()
    }

    fn create_device(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        queue_infos: &[vk::DeviceQueueCreateInfo],
    ) -> ash::Device {
        let device_extensions = Self::create_device_extensions(instance, physical_device);

        let device_create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(queue_infos)
            .enabled_extension_names(&device_extensions);

        unsafe { instance.create_device(physical_device, &device_create_info, None) }.unwrap()
    }

    fn create_device_extensions(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
    ) -> Vec<*const c_char> {
        let mut extensions = Vec::new();

        let device_extensions =
            unsafe { instance.enumerate_device_extension_properties(physical_device) }.unwrap();

        extensions.retain(|ext| {
            let extension = unsafe { CStr::from_ptr(*ext) };
            let found = device_extensions.iter().any(|inst_ext| unsafe {
                CStr::from_ptr(inst_ext.extension_name.as_ptr()) == extension
            });
            if found {
                true
            } else {
                panic!(
                    "Unable to find device extension: {}",
                    extension.to_string_lossy()
                );
            }
        });

        extensions
    }
}

#[cfg(feature = "tracing")]
unsafe extern "system" fn debug_utils_callback(
    message_severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_types: vk::DebugUtilsMessageTypeFlagsEXT,
    p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    _p_user_data: *mut std::ffi::c_void,
) -> vk::Bool32 {
    if std::thread::panicking() {
        return vk::FALSE;
    }

    let message = CStr::from_ptr((*p_callback_data).p_message);
    let ty = format!("{:?}", message_types);

    match message_severity {
        vk::DebugUtilsMessageSeverityFlagsEXT::ERROR => {
            panic!("{} - {:?}", ty, message)
        }
        vk::DebugUtilsMessageSeverityFlagsEXT::WARNING => {
            panic!("{} - {:?}", ty, message)
        }
        vk::DebugUtilsMessageSeverityFlagsEXT::INFO => {
            info!("{} - {:?}", ty, message)
        }
        vk::DebugUtilsMessageSeverityFlagsEXT::VERBOSE => {
            debug!("{} - {:?}", ty, message)
        }
        _ => {
            panic!("{} - {:?}", ty, message);
        }
    }

    vk::FALSE
}
