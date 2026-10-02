// SPDX-License-Identifier: GPL-3.0-or-later
//! Material shading models and mesh drawing.
//!
//! [`draw_mesh`] draws every submesh of a [`Mesh`] with its [`Material`]
//! using one of the [`ShadingModel`]s. The individual shaders are public, so
//! they can also be used directly with [`Renderer::draw_indexed`].

mod blinn_phong;
mod lambert;
mod unlit;

pub use blinn_phong::BlinnPhongShader;
pub use lambert::LambertShader;
pub use unlit::UnlitShader;

use crate::color::LinearRgba;
use crate::math::{Mat3, Mat4, Vec3};
use crate::pipeline::{DrawError, DrawStats, RenderState, Renderer};
use crate::raster::CullMode;
use crate::scene::{Camera, Material, Mesh, Scene};
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
#[non_exhaustive]
pub enum ShadingModel {
    /// The material's base colour, ignoring lights ("fully self-illuminated").
    #[default]
    Unlit,
    /// Diffuse lighting ([`LambertShader`]); needs vertex normals.
    Lambert,
    /// Diffuse plus specular highlights ([`BlinnPhongShader`]); needs vertex
    /// normals.
    BlinnPhong,
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
    /// Model-to-world.
    world: Mat4,
    /// Camera position in world space.
    eye: Vec3,
}

impl Transforms {
    fn new(camera: &Camera, aspect: f32, world: Mat4) -> Self {
        Self {
            mvp: camera.view_projection(aspect) * world,
            world,
            eye: camera.position(),
        }
    }

    /// The matrix transforming normals of `world`: the inverse transpose of
    /// its upper 3×3 (a singular matrix falls back to the 3×3 itself).
    fn normal_matrix(world: Mat4) -> Mat3 {
        let m = Mat3::from_mat4(world);
        let det = m.determinant();
        if det.abs() > f32::EPSILON {
            m.inverse().transpose()
        } else {
            m
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
            ShadingModel::Lambert => {
                let shader = LambertShader::new(
                    transforms.mvp,
                    transforms.world,
                    mesh,
                    material,
                    params.lighting,
                )
                .ok_or(ShadeError::MissingNormals)?;
                renderer.draw_indexed(target, &sub_state, &shader, n, indices)?
            }
            ShadingModel::BlinnPhong => {
                let t = &transforms;
                let shader =
                    BlinnPhongShader::new(t.mvp, t.world, t.eye, mesh, material, params.lighting)
                        .ok_or(ShadeError::MissingNormals)?;
                renderer.draw_indexed(target, &sub_state, &shader, n, indices)?
            }
        };
        total += stats;
    }
    Ok(total)
}

/// Draws every mesh node of `scene`, each placed by its entry in `world`
/// (from [`Scene::world_matrices`] or an animated
/// [`crate::anim::Pose::world`]). Stats are summed over all meshes.
///
/// # Errors
///
/// Stops at the first mesh that fails; see [`draw_mesh`].
///
/// # Panics
///
/// Panics if `world` does not have one matrix per node.
pub fn draw_scene(
    renderer: &mut Renderer,
    target: &mut Framebuffer,
    state: &RenderState,
    params: &DrawParams<'_>,
    scene: &Scene,
    world: &[Mat4],
) -> Result<DrawStats, ShadeError> {
    assert_eq!(
        world.len(),
        scene.nodes().len(),
        "one world matrix per node"
    );
    let mut total = DrawStats::default();
    for (node, m) in scene.nodes().iter().zip(world) {
        if let Some(mesh) = node.mesh {
            total += draw_mesh(renderer, target, state, params, &scene.meshes()[mesh.0], *m)?;
        }
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
    fn draw_scene_draws_every_mesh_node_with_its_world_matrix() {
        use crate::scene::{LocalTransform, Node, Transform};
        let mut scene = Scene::new();
        let mesh = scene.add_mesh(uv_sphere(0.5, 8, 4));
        for x in [-1.0, 1.0] {
            let t = LocalTransform::Trs(Transform {
                translation: Vec3::new(x, 0.0, 0.0),
                ..Transform::IDENTITY
            });
            scene
                .add_node(None, Node::new("s", t).with_mesh(mesh))
                .unwrap();
        }
        let mut target = Framebuffer::new(32, 32, Rgba8::BLACK).unwrap();
        let params = DrawParams {
            camera: &camera(),
            lighting: &Lighting::default(),
            model: ShadingModel::Unlit,
            materials: &[],
        };
        let stats = draw_scene(
            &mut Renderer::new(),
            &mut target,
            &RenderState::new(Viewport::new(32, 32)),
            &params,
            &scene,
            &scene.world_matrices(),
        )
        .unwrap();
        assert_eq!(stats.triangles, 2 * scene.meshes()[0].triangle_count());
        // Both spheres are visible: left and right of the centre.
        assert_eq!(target.color.get(10, 16), Some(Rgba8::WHITE));
        assert_eq!(target.color.get(22, 16), Some(Rgba8::WHITE));
        assert_eq!(target.color.get(16, 16), Some(Rgba8::BLACK));
    }

    #[test]
    fn lit_models_require_normals() {
        let bare = Mesh::new(
            uv_sphere(1.0, 8, 4).positions().to_vec(),
            uv_sphere(1.0, 8, 4).indices().to_vec(),
        )
        .unwrap();
        let mut target = Framebuffer::new(16, 16, Rgba8::BLACK).unwrap();
        let params = DrawParams {
            camera: &camera(),
            lighting: &Lighting::default(),
            model: ShadingModel::Lambert,
            materials: &[],
        };
        let err = draw_mesh(
            &mut Renderer::new(),
            &mut target,
            &RenderState::new(Viewport::new(16, 16)),
            &params,
            &bare,
            Mat4::IDENTITY,
        );
        assert_eq!(err, Err(ShadeError::MissingNormals));
        assert!(
            target.color.pixels().iter().all(|&p| p == Rgba8::BLACK),
            "nothing drawn"
        );
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
