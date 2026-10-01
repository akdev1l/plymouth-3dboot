// SPDX-License-Identifier: GPL-3.0-or-later
//! Material shading models and mesh drawing.
//!
//! [`draw_mesh`] draws every submesh of a [`Mesh`] with its [`Material`]
//! using one of the [`ShadingModel`]s. The individual shaders are public, so
//! they can also be used directly with [`Renderer::draw_indexed`].

mod unlit;

pub use unlit::UnlitShader;

use crate::color::LinearRgba;
use crate::math::{Mat4, Vec3};
use crate::pipeline::{DrawError, DrawStats, RenderState, Renderer};
use crate::raster::CullMode;
use crate::scene::{Camera, Material, Mesh};
use crate::target::Framebuffer;

/// A light infinitely far away, shining in one direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectionalLight {
    /// Direction the light travels (from the light towards the scene),
    /// in world space. Need not be normalized.
    pub direction: Vec3,
    /// Light colour times intensity, in linear light.
    pub color: LinearRgba,
}

/// The lights of a scene.
#[derive(Clone, Debug, PartialEq)]
pub struct Lighting {
    /// Light reaching every surface from all directions.
    pub ambient: LinearRgba,
    /// Directional lights.
    pub lights: Vec<DirectionalLight>,
}

impl Default for Lighting {
    /// Dim ambient light plus one white key light from the upper left front.
    fn default() -> Self {
        Self {
            ambient: LinearRgba::rgb(0.15, 0.15, 0.15),
            lights: vec![DirectionalLight {
                direction: Vec3::new(0.5, -1.0, -0.7),
                color: LinearRgba::rgb(1.0, 1.0, 1.0),
            }],
        }
    }
}

/// How surfaces respond to light.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShadingModel {
    /// The material's base colour, ignoring lights ("fully self-illuminated").
    #[default]
    Unlit,
}

/// Errors from [`draw_mesh`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ShadeError {
    /// The shading model needs vertex normals and the mesh has none.
    #[error("the shading model needs vertex normals but the mesh has none")]
    MissingNormals,
    /// The draw call itself failed.
    #[error(transparent)]
    Draw(#[from] DrawError),
}

/// Everything besides the mesh that a mesh draw needs.
#[derive(Clone, Copy, Debug)]
pub struct DrawParams<'a> {
    /// The viewing camera.
    pub camera: &'a Camera,
    /// Scene lights (ignored by unlit shading).
    pub lighting: &'a Lighting,
    /// Shading model.
    pub model: ShadingModel,
    /// Materials, indexed by the mesh's submesh material ids. Missing
    /// entries fall back to [`Material::default`].
    pub materials: &'a [Material],
}

/// Per-draw values shared by all shaders.
#[derive(Clone, Copy, Debug)]
struct Transforms {
    /// Model-view-projection.
    mvp: Mat4,
}

impl Transforms {
    fn new(camera: &Camera, aspect: f32, world: Mat4) -> Self {
        Self {
            mvp: camera.view_projection(aspect) * world,
        }
    }
}

/// Draws all submeshes of `mesh`, placed in the world by `world`.
///
/// Culling comes from `state`, except that double-sided materials are never
/// culled. Stats are summed over submeshes.
///
/// # Errors
///
/// Returns [`ShadeError::MissingNormals`] if a lit model is requested for a
/// mesh without normals, and [`ShadeError::Draw`] if a draw call fails
/// (e.g. an oversized viewport); nothing is drawn in either case.
pub fn draw_mesh(
    renderer: &mut Renderer,
    target: &mut Framebuffer,
    state: &RenderState,
    params: &DrawParams<'_>,
    mesh: &Mesh,
    world: Mat4,
) -> Result<DrawStats, ShadeError> {
    let vp = state.viewport;
    #[allow(clippy::cast_precision_loss)]
    let aspect = if vp.height == 0 {
        1.0
    } else {
        vp.width as f32 / vp.height as f32
    };
    let transforms = Transforms::new(params.camera, aspect, world);
    let fallback = Material::default();
    let mut total = DrawStats::default();
    for submesh in mesh.submeshes() {
        let material = params
            .materials
            .get(submesh.material.0)
            .unwrap_or(&fallback);
        let mut sub_state = *state;
        if material.double_sided {
            sub_state.cull = CullMode::None;
        }
        let indices = &mesh.indices()[submesh.indices.clone()];
        let n = mesh.positions().len();
        let stats = match params.model {
            ShadingModel::Unlit => {
                let shader = UnlitShader::new(transforms.mvp, mesh.positions(), material);
                renderer.draw_indexed(target, &sub_state, &shader, n, indices)?
            }
        };
        total += stats;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba8;
    use crate::math::Viewport;
    use crate::scene::primitives::uv_sphere;
    use crate::scene::{MaterialId, Projection, Submesh};

    pub(super) fn camera() -> Camera {
        Camera::look_at(
            Vec3::new(0.0, 0.0, 4.0),
            Vec3::ZERO,
            Vec3::Y,
            Projection::Perspective {
                fov_y: 0.8,
                z_near: 0.5,
                z_far: 10.0,
            },
        )
    }

    #[test]
    fn draw_mesh_uses_each_submesh_material() {
        let sphere = uv_sphere(1.0, 16, 8);
        let total = sphere.indices().len();
        let half = total / 2 / 3 * 3;
        let sphere = sphere
            .with_submeshes(vec![
                Submesh {
                    material: MaterialId(0),
                    indices: 0..half,
                },
                Submesh {
                    material: MaterialId(1),
                    indices: half..total,
                },
            ])
            .unwrap();
        let materials = [
            Material::with_color("a", LinearRgba::rgb(1.0, 0.0, 0.0)),
            Material::with_color("b", LinearRgba::rgb(0.0, 0.0, 1.0)),
        ];
        let mut target = Framebuffer::new(32, 32, Rgba8::BLACK).unwrap();
        let params = DrawParams {
            camera: &camera(),
            lighting: &Lighting::default(),
            model: ShadingModel::Unlit,
            materials: &materials,
        };
        let stats = draw_mesh(
            &mut Renderer::new(),
            &mut target,
            &RenderState::new(Viewport::new(32, 32)),
            &params,
            &sphere,
            Mat4::IDENTITY,
        )
        .unwrap();
        assert_eq!(stats.triangles, sphere.triangle_count());
        let px = target.color.pixels();
        assert!(
            px.contains(&Rgba8::new(255, 0, 0, 255)) && px.contains(&Rgba8::new(0, 0, 255, 255))
        );
    }

    #[test]
    fn missing_materials_fall_back_to_default() {
        let sphere = uv_sphere(1.0, 8, 4);
        let mut target = Framebuffer::new(16, 16, Rgba8::BLACK).unwrap();
        let params = DrawParams {
            camera: &camera(),
            lighting: &Lighting::default(),
            model: ShadingModel::Unlit,
            materials: &[],
        };
        draw_mesh(
            &mut Renderer::new(),
            &mut target,
            &RenderState::new(Viewport::new(16, 16)),
            &params,
            &sphere,
            Mat4::IDENTITY,
        )
        .unwrap();
        assert_eq!(target.color.get(8, 8), Some(Rgba8::WHITE));
    }

    #[test]
    fn double_sided_materials_are_not_culled() {
        // Viewed from inside, a sphere shows only back faces.
        let sphere = uv_sphere(5.0, 16, 8);
        let inside = Camera::look_at(
            Vec3::ZERO,
            Vec3::NEG_Z,
            Vec3::Y,
            Projection::Perspective {
                fov_y: 1.0,
                z_near: 0.1,
                z_far: 20.0,
            },
        );
        let mut state = RenderState::new(Viewport::new(16, 16));
        state.cull = CullMode::Back;
        for (double_sided, expect_drawn) in [(false, false), (true, true)] {
            let materials = [Material {
                double_sided,
                ..Material::default()
            }];
            let params = DrawParams {
                camera: &inside,
                lighting: &Lighting::default(),
                model: ShadingModel::Unlit,
                materials: &materials,
            };
            let mut target = Framebuffer::new(16, 16, Rgba8::BLACK).unwrap();
            let stats = draw_mesh(
                &mut Renderer::new(),
                &mut target,
                &state,
                &params,
                &sphere,
                Mat4::IDENTITY,
            )
            .unwrap();
            assert_eq!(
                stats.fragments_written > 0,
                expect_drawn,
                "double_sided {double_sided}: {stats:?}"
            );
        }
    }
}
