/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! An implementation of layer management using surfman

use euclid::Point2D;
use euclid::Rect;
use euclid::Size2D;

use sparkle::gl::Gl;

use std::collections::HashMap;

use surfman::Context as SurfmanContext;
use surfman::Device as SurfmanDevice;
use surfman::SurfaceAccess;
use surfman::SurfaceTexture;

use surfman_chains::SwapChains;
use surfman_chains::SwapChainsAPI;

use webxr_api::ContextId;
use webxr_api::Error;
use webxr_api::GLContexts;
use webxr_api::GLTypes;
use webxr_api::LayerId;
use webxr_api::LayerInit;
use webxr_api::LayerManagerAPI;
use webxr_api::SubImage;
use webxr_api::SubImages;
use webxr_api::Viewports;

#[derive(Copy, Clone, Debug)]
pub enum SurfmanGL {}

impl GLTypes for SurfmanGL {
    type Device = SurfmanDevice;
    type Context = SurfmanContext;
    type Bindings = Gl;
}

pub struct SurfmanLayerManager {
    layers: Vec<(ContextId, LayerId)>,
    swap_chains: SwapChains<LayerId, SurfmanDevice>,
    textures: HashMap<LayerId, SurfaceTexture>,
    viewports: Viewports,
}

impl SurfmanLayerManager {
    pub fn new(
        viewports: Viewports,
        swap_chains: SwapChains<LayerId, SurfmanDevice>,
    ) -> SurfmanLayerManager {
        let layers = Vec::new();
        let textures = HashMap::new();
        SurfmanLayerManager {
            layers,
            swap_chains,
            textures,
            viewports,
        }
    }
}

impl LayerManagerAPI<SurfmanGL> for SurfmanLayerManager {
    fn create_layer(
        &mut self,
        device: &mut SurfmanDevice,
        context: &mut SurfmanContext,
        context_id: ContextId,
        init: LayerInit,
    ) -> Result<LayerId, Error> {
        let texture_size = init.texture_size(&self.viewports);
        let layer_id = LayerId::new();
        let access = SurfaceAccess::GPUOnly;
        let size = texture_size.to_untyped();
        self.swap_chains
            .create_detached_swap_chain(layer_id, size, device, context, access)
            .map_err(|err| Error::BackendSpecific(format!("{:?}", err)))?;
        self.layers.push((context_id, layer_id));
        Ok(layer_id)
    }

    fn destroy_layer(
        &mut self,
        device: &mut SurfmanDevice,
        context: &mut SurfmanContext,
        context_id: ContextId,
        layer_id: LayerId,
    ) {
        self.layers.retain(|&ids| ids != (context_id, layer_id));
        let _ = self.swap_chains.destroy(layer_id, device, context);
        self.textures.remove(&layer_id);
    }

    fn layers(&self) -> &[(ContextId, LayerId)] {
        &self.layers[..]
    }

    fn begin_frame(
        &mut self,
        device: &mut SurfmanDevice,
        contexts: &mut dyn GLContexts<SurfmanGL>,
        layers: &[(ContextId, LayerId)],
    ) -> Result<Vec<SubImages>, Error> {
        layers
            .iter()
            .map(|&(context_id, layer_id)| {
                let context = contexts
                    .context(device, context_id)
                    .ok_or(Error::NoMatchingDevice)?;
                let swap_chain = self
                    .swap_chains
                    .get(layer_id)
                    .ok_or(Error::NoMatchingDevice)?;
                let surface_size = Size2D::from_untyped(swap_chain.size());
                let surface_texture = swap_chain
                    .take_surface_texture(device, context)
                    .map_err(|_| Error::NoMatchingDevice)?;
                let color_texture = device.surface_texture_object(&surface_texture);
                let depth_stencil_texture = None;
                let texture_array_index = None;
                let origin = Point2D::new(0, 0);
                let sub_image = Some(SubImage {
                    color_texture,
                    depth_stencil_texture,
                    texture_array_index,
                    viewport: Rect::new(origin, surface_size),
                });
                let view_sub_images = self
                    .viewports
                    .viewports
                    .iter()
                    .map(|&viewport| SubImage {
                        color_texture,
                        depth_stencil_texture,
                        texture_array_index,
                        viewport,
                    })
                    .collect();
                self.textures.insert(layer_id, surface_texture);
                Ok(SubImages {
                    layer_id,
                    sub_image,
                    view_sub_images,
                })
            })
            .collect()
    }

    fn end_frame(
        &mut self,
        device: &mut SurfmanDevice,
        contexts: &mut dyn GLContexts<SurfmanGL>,
        layers: &[(ContextId, LayerId)],
    ) -> Result<(), Error> {
        for &(context_id, layer_id) in layers {
            let gl = contexts
                .bindings(device, context_id)
                .ok_or(Error::NoMatchingDevice)?;
            gl.flush();
            let context = contexts
                .context(device, context_id)
                .ok_or(Error::NoMatchingDevice)?;
            let surface_texture = self
                .textures
                .remove(&layer_id)
                .ok_or(Error::NoMatchingDevice)?;
            let swap_chain = self
                .swap_chains
                .get(layer_id)
                .ok_or(Error::NoMatchingDevice)?;
            swap_chain
                .recycle_surface_texture(device, context, surface_texture)
                .map_err(|err| Error::BackendSpecific(format!("{:?}", err)))?;
            swap_chain
                .swap_buffers(device, context)
                .map_err(|err| Error::BackendSpecific(format!("{:?}", err)))?;
        }
        Ok(())
    }
}
