// Copyright (C) 2023 Vladimir Kuskov

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

use arrayvec::ArrayVec;
use ash::{extensions::khr, vk};
use gpu_alloc::{Dedicated, Request};
use gpu_alloc_ash::{device_properties, AshMemoryDevice};
use gpu_descriptor_ash::AshDescriptorDevice;
use kiri_core::{Handle, Pool};
use parking_lot::{Mutex, RwLock};
use std::ffi::{CStr, CString};
use std::sync::Arc;
use std::{mem, slice};

use crate::{RenderError, RenderResult};

use super::{
    frame::Frame, Buffer, DescriptorAllocator, DropList, GpuAllocator, GpuMemory, Image, Instance,
    PhysicalDevice, ToDrop, UniformStorage,
};

pub type ImageHandle = Handle<vk::Image, Image>;
pub type BufferHandle = Handle<vk::Buffer, Buffer>;
pub struct BufferSlice(BufferHandle, u32);

pub type ImageStorage = Pool<vk::Image, Image>;
pub type BufferStorage = Pool<vk::Buffer, Buffer>;

pub struct CommandBuffer {
    pub raw: vk::CommandBuffer,
    pub fence: vk::Fence,
}

impl CommandBuffer {
    pub fn primary(device: &ash::Device, pool: vk::CommandPool) -> RenderResult<Self> {
        let cb = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::builder()
                    .command_buffer_count(1)
                    .command_pool(pool)
                    .level(vk::CommandBufferLevel::PRIMARY),
            )
        }?[0];
        let fence = unsafe {
            device.create_fence(
                &vk::FenceCreateInfo::builder()
                    .flags(vk::FenceCreateFlags::SIGNALED)
                    .build(),
                None,
            )?
        };
        Ok(Self { raw: cb, fence })
    }

    pub fn free(&self, device: &ash::Device) {
        unsafe {
            device
                .wait_for_fences(slice::from_ref(&self.fence), true, u64::MAX)
                .unwrap();
            device.destroy_fence(self.fence, None);
            // Command buffer itself is freed by pool.
        }
    }
}

pub struct Device {
    pub(crate) instance: Instance,
    pub(crate) raw: ash::Device,
    pub(crate) pdevice: PhysicalDevice,
    pub(crate) memory_allocator: Mutex<GpuAllocator>,
    pub(crate) descriptor_allocator: Mutex<DescriptorAllocator>,
    pub(crate) image_storage: RwLock<ImageStorage>,
    pub(crate) buffer_storage: RwLock<BufferStorage>,
    pub(crate) uniform_storage: Mutex<UniformStorage>,
    pub(crate) current_drop_list: Mutex<DropList>,
    frames: [Mutex<Arc<Frame>>; 2],
}

impl Device {
    pub fn new(instance: Instance, pdevice: PhysicalDevice) -> RenderResult<Self> {
        if !pdevice.is_queue_flag_supported(
            vk::QueueFlags::GRAPHICS | vk::QueueFlags::TRANSFER | vk::QueueFlags::COMPUTE,
        ) {
            return Err(RenderError::NoSuitableDevice);
        };

        let device_extension_names = vec![
            khr::Swapchain::name().as_ptr(),
            vk::KhrImageFormatListFn::name().as_ptr(),
            vk::KhrImagelessFramebufferFn::name().as_ptr(),
        ];

        for ext in &device_extension_names {
            let ext = unsafe { CStr::from_ptr(*ext).to_str() }.unwrap();
            if !pdevice.is_extensions_sipported(ext) {
                return Err(RenderError::ExtensionNotFound(ext.into()));
            }
        }

        let universal_queue = pdevice
            .get_queue(
                vk::QueueFlags::GRAPHICS | vk::QueueFlags::TRANSFER | vk::QueueFlags::COMPUTE,
            )
            .ok_or(RenderError::NoSuitableQueue)?;

        let mut imageless_frame_buffer = vk::PhysicalDeviceImagelessFramebufferFeatures::default();

        let mut features = vk::PhysicalDeviceFeatures2::builder()
            .push_next(&mut imageless_frame_buffer)
            .build();

        unsafe {
            instance
                .raw
                .get_physical_device_features2(pdevice.raw, &mut features)
        };

        let mut priorities = ArrayVec::<_, 3>::new();
        if universal_queue.properties.queue_count < 3 {
            priorities.push(1.0);
        } else {
            (0..3).for_each(|_| priorities.push(1.0));
        }

        let queue_info = [vk::DeviceQueueCreateInfo::builder()
            .queue_family_index(universal_queue.index)
            .queue_priorities(&priorities)
            .build()];

        let device_create_info = vk::DeviceCreateInfo::builder()
            .queue_create_infos(&queue_info)
            .enabled_extension_names(&device_extension_names)
            .push_next(&mut features)
            .build();

        let device = unsafe {
            instance
                .raw
                .create_device(pdevice.raw, &device_create_info, None)?
        };

        let allocator_config = gpu_alloc::Config {
            dedicated_threshold: 64 * 1024 * 1024,
            preferred_dedicated_threshold: 32 * 1024 * 1024,
            transient_dedicated_threshold: 32 * 1024 * 1024,
            final_free_list_chunk: 1024 * 1024,
            minimal_buddy_size: 128,
            starting_free_list_chunk: 64 * 1024,
            initial_buddy_dedicated_size: 32 * 1024 * 1024,
        };
        let allocator_props =
            unsafe { device_properties(&instance.raw, Instance::vulkan_version(), pdevice.raw) }?;
        let mut memory_allocator = GpuAllocator::new(allocator_config, allocator_props);
        let descriptor_allocator = DescriptorAllocator::new(0);

        let frames = [
            Mutex::new(Arc::new(Frame::new(
                &device,
                &mut memory_allocator,
                universal_queue.index,
            )?)),
            Mutex::new(Arc::new(Frame::new(
                &device,
                &mut memory_allocator,
                universal_queue.index,
            )?)),
        ];

        let uniform_storage = UniformStorage::new(&device, &mut memory_allocator)?;

        Ok(Self {
            instance,
            raw: device,
            pdevice,
            memory_allocator: Mutex::new(memory_allocator),
            descriptor_allocator: Mutex::new(descriptor_allocator),
            image_storage: RwLock::default(),
            buffer_storage: RwLock::default(),
            frames,
            uniform_storage: Mutex::new(uniform_storage),
            current_drop_list: Mutex::default(),
        })
    }

    pub fn begin_frame(&self) -> RenderResult<Arc<Frame>> {
        puffin::profile_function!();
        let mut frame0 = self.frames[0].lock();
        {
            let frame0 = Arc::get_mut(&mut frame0)
                .expect("Unable to begin frame: frame data is being held by user code");
            unsafe {
                self.raw.wait_for_fences(
                    &[frame0.present_cb.fence, frame0.main_cb.fence],
                    true,
                    u64::MAX,
                )
            }?;
            let mut memory_allocator = self.memory_allocator.lock();
            let mut descriptor_allocator = self.descriptor_allocator.lock();
            let mut uniforms = self.uniform_storage.lock();
            frame0.reset(
                &self.raw,
                &mut memory_allocator,
                &mut descriptor_allocator,
                &mut uniforms,
            )?;
            frame0
                .drop_list
                .replace(mem::take(&mut self.current_drop_list.lock()));
        }
        Ok(frame0.clone())
    }

    pub fn end_frame(&self, frame: Arc<Frame>) {
        drop(frame);

        let mut frame0 = self.frames[0].lock();
        {
            let frame0 = Arc::get_mut(&mut frame0)
                .expect("Unable to finish frame: frame data is being held by user code");
            let mut frame1 = self.frames[1].lock();
            let frame1 = Arc::get_mut(&mut frame1).unwrap();
            std::mem::swap(frame0, frame1);
        }
    }

    pub(crate) fn allocate_impl(
        device: &ash::Device,
        allocator: &mut GpuAllocator,
        requirements: vk::MemoryRequirements,
        location: gpu_alloc::UsageFlags,
        dedicated: bool,
    ) -> RenderResult<GpuMemory> {
        let request = Request {
            size: requirements.size,
            align_mask: requirements.alignment,
            usage: location,
            memory_types: requirements.memory_type_bits,
        };

        Ok(if dedicated {
            unsafe {
                allocator.alloc_with_dedicated(
                    AshMemoryDevice::wrap(device),
                    request,
                    Dedicated::Required,
                )
            }
        } else {
            unsafe { allocator.alloc(AshMemoryDevice::wrap(device), request) }
        }?)
    }

    pub fn set_object_name<T: vk::Handle>(&self, object: T, name: &str) {
        Self::set_object_name_impl(&self.instance, &self.raw, object, name);
    }

    pub(crate) fn set_object_name_impl<T: vk::Handle>(
        instance: &Instance,
        device: &ash::Device,
        object: T,
        name: &str,
    ) {
        if let Some(debug_utils) = instance.get_debug_utils() {
            let name = CString::new(name).unwrap();
            let name_info = vk::DebugUtilsObjectNameInfoEXT::builder()
                .object_type(T::TYPE)
                .object_handle(object.as_raw())
                .object_name(&name)
                .build();
            unsafe { debug_utils.set_debug_utils_object_name(device.handle(), &name_info) }
                .unwrap();
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe { self.raw.device_wait_idle() }.unwrap();
        let mut memory_allocator = self.memory_allocator.lock();
        let mut descriptor_allocator = self.descriptor_allocator.lock();
        let mut uniform_storage = self.uniform_storage.lock();

        let mut drop_list = DropList::default();
        let mut images = self.image_storage.write().drain().collect::<Vec<_>>();
        let mut buffers = self.buffer_storage.write().drain().collect::<Vec<_>>();
        images.iter_mut().for_each(|x| x.1.to_drop(&mut drop_list));
        buffers.iter_mut().for_each(|x| x.1.to_drop(&mut drop_list));

        drop_list.purge(
            &self.raw,
            &mut memory_allocator,
            &mut descriptor_allocator,
            &mut uniform_storage,
        );

        self.frames.iter().for_each(|x| {
            Arc::get_mut(&mut x.lock())
                .expect("Frame data shouldn't be kept by anybody else")
                .free(
                    &self.raw,
                    &mut memory_allocator,
                    &mut descriptor_allocator,
                    &mut uniform_storage,
                )
        });

        uniform_storage.free(&self.raw, &mut memory_allocator);

        unsafe {
            descriptor_allocator.cleanup(AshDescriptorDevice::wrap(&self.raw));
            memory_allocator.cleanup(AshMemoryDevice::wrap(&self.raw));
            self.raw.destroy_device(None);
        }
    }
}
