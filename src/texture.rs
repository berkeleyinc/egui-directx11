// This file contains implementations inspired by or derived from the following
// sources:
// - https://github.com/ohchase/egui-directx/blob/master/egui-directx11/src/texture.rs
//
// Here I would express my gratitude for their contributions to the Rust
// community. Their work served as a valuable reference and inspiration for this
// project.
//
// Nekomaru, March 2024

use std::{collections::HashMap, mem, slice};

use egui::{Color32, ImageData, TextureId, TexturesDelta};

use windows::{
    Win32::Graphics::{Direct3D11::*, Dxgi::Common::*},
    core::Result,
};

struct Texture {
    tex: ID3D11Texture2D,
    srv: ID3D11ShaderResourceView,
    pixels: Vec<Color32>,
    width: usize,
}

pub struct TexturePool {
    device: ID3D11Device,
    pool: HashMap<TextureId, Texture>,
}

impl TexturePool {
    pub fn new(device: &ID3D11Device) -> Self {
        Self {
            device: device.clone(),
            pool: HashMap::new(),
        }
    }

    pub fn get_srv(&self, tid: TextureId) -> Option<ID3D11ShaderResourceView> {
        self.pool.get(&tid).map(|t| t.srv.clone())
    }

    pub fn update(
        &mut self,
        ctx: &ID3D11DeviceContext,
        mut delta: TexturesDelta,
    ) -> Result<()> {
        // `set` maps each texture to a list of deltas: a whole-texture upload
        // replaces the entry, while partials apply in order. Consume the set so
        // the `Drop` assert on `TexturesDelta` stays quiet.
        //
        // The work is done in a closure so `delta` is always cleared, even when a
        // D3D call fails early - otherwise the unapplied remainder trips the
        // `debug_assert!` in `TexturesDelta::drop`.
        let result = (|| -> Result<()> {
            for (tid, deltas) in delta.set.clone() {
                if let Some(whole) = deltas.iter().find(|d| d.is_whole()) {
                    if whole.image.width() > 0 && whole.image.height() > 0 {
                        self.pool.insert(
                            tid,
                            Self::create_texture(
                                &self.device,
                                whole.image.clone(),
                            )?,
                        );
                        // the old texture is returned and dropped here, freeing
                        // all its gpu resource.
                        continue;
                    }
                }
                for d in deltas.iter().filter(|d| !d.is_whole()) {
                    match self.pool.get_mut(&tid) {
                        Some(tex) => {
                            Self::update_partial(
                                ctx,
                                tex,
                                d.image.clone(),
                                d.pos.unwrap(),
                            )?;
                        },
                        None => log::warn!(
                            "egui wants to update a non-existing texture {tid:?}. this request will be ignored."
                        ),
                    }
                }
            }
            for tid in delta.free.iter() {
                self.pool.remove(tid);
            }
            Ok(())
        })();
        delta.clear();
        result
    }

    fn update_partial(
        ctx: &ID3D11DeviceContext,
        old: &mut Texture,
        image: ImageData,
        [nx, ny]: [usize; 2],
    ) -> Result<()> {
        let subr = unsafe {
            let mut output = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(
                &old.tex,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut output),
            )?;
            output
        };
        match image {
            ImageData::Color(f) => {
                let data = unsafe {
                    let slice = slice::from_raw_parts_mut(
                        subr.pData as *mut Color32,
                        old.pixels.len(),
                    );
                    slice.as_mut_ptr().copy_from_nonoverlapping(
                        old.pixels.as_ptr(),
                        old.pixels.len(),
                    );
                    slice
                };

                for y in 0..f.height() {
                    for x in 0..f.width() {
                        let whole = (ny + y) * old.width + nx + x;
                        let frac = y * f.width() + x;
                        old.pixels[whole] = f.pixels[frac];
                        data[whole] = f.pixels[frac];
                    }
                }
            },
        }
        unsafe { ctx.Unmap(&old.tex, 0) };
        Ok(())
    }

    fn create_texture(
        device: &ID3D11Device,
        data: ImageData,
    ) -> Result<Texture> {
        let width = data.width();

        let pixels = match &data {
            ImageData::Color(c) => c.pixels.clone(),
        };

        let desc = D3D11_TEXTURE2D_DESC {
            Width: data.width() as _,
            Height: data.height() as _,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as _,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as _,
            ..Default::default()
        };

        let subresource_data = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr() as _,
            SysMemPitch: (width * mem::size_of::<Color32>()) as u32,
            SysMemSlicePitch: 0,
        };

        let mut tex = None;
        unsafe {
            device.CreateTexture2D(
                &desc,
                Some(&subresource_data),
                Some(&mut tex),
            )
        }?;
        let tex = tex.unwrap();

        let mut srv = None;
        unsafe { device.CreateShaderResourceView(&tex, None, Some(&mut srv)) }?;
        let srv = srv.unwrap();

        Ok(Texture {
            tex,
            srv,
            width,
            pixels,
        })
    }
}
