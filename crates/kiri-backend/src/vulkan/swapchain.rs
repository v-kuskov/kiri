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

use std::slice;

use arrayvec::ArrayVec;
use ash::{
    extensions::khr,
    vk::{self},
};
use log::info;
use parking_lot::RwLock;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::{
    vulkan::{Image, ImageDesc},
    RenderError, RenderResult,
};

use super::{physical_device::PhysicalDevice, Device, Instance};

const DESIRED_IMAGES_COUNT: usize = 3;

pub struct Surface {
    pub raw: vk::SurfaceKHR,
    pub loader: khr::Surface,
}

impl Surface {
    pub fn new(
        instance: &Instance,
        display_handle: RawDisplayHandle,
        window_handle: RawWindowHandle,
    ) -> RenderResult<Self> {
        let surface = unsafe {
            ash_window::create_surface(
                &instance.entry,
                &instance.raw,
                display_handle,
                window_handle,
                None,
            )
        }?;
        let loader = khr::Surface::new(&instance.entry, &instance.raw);

        Ok(Self {
            raw: surface,
            loader,
        })
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe { self.loader.destroy_surface(self.raw, None) };
    }
}

pub struct Swapchain {
    pub surface: Surface,
    pub raw: vk::SwapchainKHR,
    pub images: ArrayVec<Image, DESIRED_IMAGES_COUNT>,
    pub loader: khr::Swapchain,
    pub acquire_semaphores: ArrayVec<vk::Semaphore, DESIRED_IMAGES_COUNT>,
    pub rendering_finished_semaphores: ArrayVec<vk::Semaphore, DESIRED_IMAGES_COUNT>,
    pub next_semaphore: usize,
    pub dims: [u32; 2],
    pub format: vk::Format,
}

pub struct SwapchainImage<'a> {
    pub image: &'a Image,
    pub image_index: u32,
    pub acquire_semaphore: vk::Semaphore,
    pub rendering_finished: vk::Semaphore,
}

pub enum AcquiredSurface<'a> {
    NeedRecreate,
    Image(SwapchainImage<'a>),
}

impl Swapchain {
    pub fn new(device: &Device, surface: Surface, resolution: [u32; 2]) -> RenderResult<Self> {
        info!(
            "Create swapchain for resolution {} x {}",
            resolution[0], resolution[1]
        );
        let surface_capabilities = unsafe {
            surface
                .loader
                .get_physical_device_surface_capabilities(device.pdevice.raw, surface.raw)
        }?;

        let formats = Self::enumerate_surface_formats(&device.pdevice, &surface)?;
        let format = match Self::select_surface_format(&formats) {
            Some(format) => format,
            None => return Err(RenderError::NotSupported),
        };

        let mut desired_image_count =
            (DESIRED_IMAGES_COUNT as u32).max(surface_capabilities.min_image_count);
        if surface_capabilities.max_image_count != 0 {
            desired_image_count = desired_image_count.min(surface_capabilities.max_image_count);
        }

        info!("Swapchain image count {}", desired_image_count);

        let surface_resolution = match surface_capabilities.current_extent.width {
            u32::MAX => vk::Extent2D {
                width: resolution[0],
                height: resolution[1],
            },
            _ => surface_capabilities.current_extent,
        };

        if surface_resolution.width == 0 || surface_resolution.height == 0 {
            panic!("Can't create swachain for surface with zero size");
        }

        let present_mode_preferences = [vk::PresentModeKHR::FIFO_RELAXED, vk::PresentModeKHR::FIFO];

        let present_modes = unsafe {
            surface
                .loader
                .get_physical_device_surface_present_modes(device.pdevice.raw, surface.raw)
        }?;

        info!("Swapchain format: {:?}", format.format);

        let present_mode = present_mode_preferences
            .into_iter()
            .find(|mode| present_modes.contains(mode))
            .unwrap_or(vk::PresentModeKHR::FIFO);

        info!("Presentation mode: {:?}", present_mode);

        let pre_transform = if surface_capabilities
            .supported_transforms
            .contains(vk::SurfaceTransformFlagsKHR::IDENTITY)
        {
            vk::SurfaceTransformFlagsKHR::IDENTITY
        } else {
            surface_capabilities.current_transform
        };

        let swapchain_create_info = vk::SwapchainCreateInfoKHR::builder()
            .surface(surface.raw)
            .min_image_count(desired_image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(surface_resolution)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(pre_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true)
            .image_array_layers(1)
            .build();

        let loader = khr::Swapchain::new(&device.instance.raw, &device.raw);
        let swapchain = unsafe { loader.create_swapchain(&swapchain_create_info, None) }?;
        let images = unsafe { loader.get_swapchain_images(swapchain) }?;
        let images = images
            .iter()
            .map(|image| Image {
                raw: *image,
                desc: ImageDesc {
                    ty: vk::ImageType::TYPE_2D,
                    usage: vk::ImageUsageFlags::COLOR_ATTACHMENT,
                    format: format.format,
                    extent: [surface_resolution.width, surface_resolution.height],
                    tiling: vk::ImageTiling::OPTIMAL,
                    mip_levels: 1,
                    array_elements: 1,
                },
                memory: None,
                views: RwLock::default(),
            })
            .enumerate()
            .map(|(index, image)| {
                device.set_object_name(image.raw, &format!("Swapchain {index}"));
                image
            })
            .collect();

        let mut acquire_semaphores = ArrayVec::new();
        let mut rendering_finished_semaphores = ArrayVec::new();
        for index in 0..desired_image_count {
            let acquire_semaphore = unsafe {
                device
                    .raw
                    .create_semaphore(&vk::SemaphoreCreateInfo::builder(), None)
            }?;
            let rendering_finished_semaphore = unsafe {
                device
                    .raw
                    .create_semaphore(&vk::SemaphoreCreateInfo::builder(), None)
            }?;
            device.set_object_name(acquire_semaphore, &format!("Acquire {index}"));
            device.set_object_name(rendering_finished_semaphore, &format!("Finished {index}"));
            acquire_semaphores.push(acquire_semaphore);
            rendering_finished_semaphores.push(rendering_finished_semaphore);
        }
        Ok(Self {
            surface,
            raw: swapchain,
            images,
            acquire_semaphores,
            rendering_finished_semaphores,
            next_semaphore: 0,
            loader,
            format: format.format,
            dims: [surface_resolution.width, surface_resolution.height],
        })
    }

    pub fn acquire_next_image(&mut self) -> RenderResult<AcquiredSurface> {
        puffin::profile_scope!("wait for swapchain");
        let acquire_semaphore = self.acquire_semaphores[self.next_semaphore];
        let rendering_finished_semaphore = self.rendering_finished_semaphores[self.next_semaphore];

        let present_index = match unsafe {
            self.loader
                .acquire_next_image(self.raw, u64::MAX, acquire_semaphore, vk::Fence::null())
        } {
            Ok((present_index, _)) => present_index,
            Err(vk::Result::SUBOPTIMAL_KHR | vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                return Ok(AcquiredSurface::NeedRecreate)
            }
            Err(err) => return Err(RenderError::from(err)),
        };

        assert_eq!(present_index as usize, self.next_semaphore);

        self.next_semaphore = (self.next_semaphore + 1) % self.images.len();
        Ok(AcquiredSurface::Image(SwapchainImage {
            image: &self.images[present_index as usize],
            image_index: present_index,
            acquire_semaphore,
            rendering_finished: rendering_finished_semaphore,
        }))
    }

    pub fn present_image(&self, image: SwapchainImage, queue: vk::Queue) {
        puffin::profile_scope!("present");
        let present_info = vk::PresentInfoKHR::builder()
            .wait_semaphores(slice::from_ref(&image.rendering_finished))
            .swapchains(slice::from_ref(&self.raw))
            .image_indices(slice::from_ref(&image.image_index))
            .build();

        match unsafe { self.loader.queue_present(queue, &present_info) } {
            Ok(_) => (),
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) | Err(vk::Result::SUBOPTIMAL_KHR) => {}
            Err(err) => panic!("Can't present image: {}", err),
        }
    }

    fn enumerate_surface_formats(
        pdevice: &PhysicalDevice,
        surface: &Surface,
    ) -> RenderResult<Vec<vk::SurfaceFormatKHR>> {
        Ok(unsafe {
            surface
                .loader
                .get_physical_device_surface_formats(pdevice.raw, surface.raw)
        }?)
    }

    fn select_surface_format(formats: &[vk::SurfaceFormatKHR]) -> Option<vk::SurfaceFormatKHR> {
        let prefered = [
            vk::SurfaceFormatKHR {
                format: vk::Format::A2B10G10R10_UNORM_PACK32,
                color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
            },
            vk::SurfaceFormatKHR {
                format: vk::Format::B8G8R8A8_UNORM,
                color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
            },
        ];

        prefered.into_iter().find(|format| formats.contains(format))
    }

    pub fn free(&mut self, device: &ash::Device) {
        unsafe { device.device_wait_idle().unwrap() }
        unsafe { self.loader.destroy_swapchain(self.raw, None) };
        for semaphore in &self.acquire_semaphores {
            unsafe { device.destroy_semaphore(*semaphore, None) }
        }
        for semaphore in &mut self.rendering_finished_semaphores {
            unsafe { device.destroy_semaphore(*semaphore, None) }
        }
        for image in self.images.iter() {
            image.clear_views(device);
        }
    }
}
