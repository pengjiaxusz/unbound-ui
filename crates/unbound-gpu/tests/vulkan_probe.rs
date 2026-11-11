//! Environment probe: does this machine expose a Vulkan 1.3 device, and which
//! of the features the renderer *requires* are actually present?
//!
//! Run with `cargo test -p unbound-gpu --test vulkan_probe -- --ignored --nocapture`.
//! The test is `#[ignore]`d by default so headless CI without a GPU stays green;
//! the renderer's own integration tests use the same capability gate.

use ash::vk;

#[test]
#[ignore = "requires a Vulkan 1.3 driver; run explicitly on a GPU machine"]
fn report_vulkan_capabilities() {
    // SAFETY: `Entry::load` only loads the loader; every Vulkan object created
    // below is destroyed before returning.
    unsafe {
        let entry = ash::Entry::load().expect("Vulkan loader present");
        let app_info = vk::ApplicationInfo::default()
            .application_name(c"unbound-probe")
            .api_version(vk::make_api_version(0, 1, 3, 0));
        let create_info = vk::InstanceCreateInfo::default().application_info(&app_info);
        let instance = entry.create_instance(&create_info, None).expect("instance");

        let devices = instance
            .enumerate_physical_devices()
            .expect("enumerate physical devices");
        println!("physical devices: {}", devices.len());
        for (i, pd) in devices.iter().enumerate() {
            let props = instance.get_physical_device_properties(*pd);
            let name = props
                .device_name_as_c_str()
                .map(|c| c.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "<unnamed>".into());
            println!(
                "  [{i}] {name} api={}.{}.{} type={:?}",
                vk::api_version_major(props.api_version),
                vk::api_version_minor(props.api_version),
                vk::api_version_patch(props.api_version),
                props.device_type
            );

            let exts = instance
                .enumerate_device_extension_properties(*pd)
                .unwrap_or_default();
            let names: Vec<String> = exts
                .iter()
                .filter_map(|e| {
                    e.extension_name_as_c_str()
                        .ok()
                        .map(|c| c.to_string_lossy().into_owned())
                })
                .collect();
            for wanted in [
                "VK_KHR_buffer_device_address",
                "VK_EXT_descriptor_indexing",
                "VK_KHR_dynamic_rendering",
                "VK_KHR_synchronization2",
                "VK_KHR_timeline_semaphore",
                "VK_KHR_draw_indirect_count",
                "VK_EXT_swapchain_maintenance1",
                "VK_KHR_present_wait",
                "VK_KHR_swapchain",
            ] {
                let present = names.iter().any(|n| n == wanted);
                println!("      {wanted}: {present}");
            }
        }
        instance.destroy_instance(None);
    }
}
