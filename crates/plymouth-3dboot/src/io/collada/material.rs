// SPDX-License-Identifier: GPL-3.0-or-later
//! `<material>` → `<effect>` → common profile shading parameters.

use roxmltree::Node;

use super::ColladaError;
use super::xml::{Document, attr, child, fixed_numbers, numbers, require};
use crate::color::LinearRgba;
use crate::scene::Material;

/// Shininess values at or below this are treated as normalized glossiness
/// (as 3ds Max exports them) and scaled to a Phong exponent.
const NORMALIZED_SHININESS_MAX: f32 = 1.0;
/// Phong exponent for normalized glossiness 1.0.
const NORMALIZED_SHININESS_SCALE: f32 = 128.0;

/// A colour parameter (`<diffuse><color>`), or `None` if absent or given as
/// a texture (with a warning).
fn color_param(
    doc: &Document<'_>,
    technique: Node<'_, '_>,
    name: &str,
    warnings: &mut Vec<String>,
) -> Result<Option<LinearRgba>, ColladaError> {
    let Some(param) = child(technique, name) else {
        return Ok(None);
    };
    if let Some(color) = child(param, "color") {
        let [r, g, b, a] = fixed_numbers::<4>(doc, color)?;
        // Authored colours are treated as sRGB-encoded (docs/conventions.md).
        let clamp = |v: f32| v.clamp(0.0, 1.0);
        return Ok(Some(LinearRgba::from_srgb(
            clamp(r),
            clamp(g),
            clamp(b),
            clamp(a),
        )));
    }
    if child(param, "texture").is_some() {
        warnings.push(format!(
            "line {}: <{name}> textures are not supported; using the colour default",
            doc.line(param)
        ));
    }
    Ok(None)
}

fn float_param(
    doc: &Document<'_>,
    technique: Node<'_, '_>,
    name: &str,
) -> Result<Option<f32>, ColladaError> {
    let Some(param) = child(technique, name) else {
        return Ok(None);
    };
    let Some(f) = child(param, "float") else {
        return Ok(None);
    };
    Ok(Some(fixed_numbers::<1>(doc, f)?[0]))
}

/// A float from `<extra><technique profile="..."><name>`, searched
/// anywhere below `node`.
fn extra_float(
    doc: &Document<'_>,
    node: Node<'_, '_>,
    profile: &str,
    name: &str,
) -> Result<Option<f32>, ColladaError> {
    for technique in node
        .descendants()
        .filter(|n| n.tag_name().name() == "technique" && n.attribute("profile") == Some(profile))
    {
        if let Some(e) = child(technique, name) {
            let target = child(e, "float").unwrap_or(e);
            let v: Vec<f32> = numbers(doc, target)?;
            return Ok(v.first().copied());
        }
    }
    Ok(None)
}

/// Converts an `<effect>` to a [`Material`] named `name`.
pub(crate) fn parse_effect(
    doc: &Document<'_>,
    effect: Node<'_, '_>,
    name: &str,
    warnings: &mut Vec<String>,
) -> Result<Material, ColladaError> {
    let mut material = Material {
        name: name.to_owned(),
        ..Material::default()
    };
    let Some(profile) = child(effect, "profile_COMMON") else {
        warnings.push(format!(
            "effect {:?}: no profile_COMMON; using a default material",
            effect.attribute("id").unwrap_or("")
        ));
        return Ok(material);
    };
    let technique = require(doc, profile, "technique")?;
    let Some(model) = ["blinn", "phong", "lambert", "constant"]
        .iter()
        .find_map(|m| child(technique, m))
    else {
        warnings.push(format!(
            "line {}: unsupported shading technique; using a default material",
            doc.line(technique)
        ));
        return Ok(material);
    };

    let diffuse = color_param(doc, model, "diffuse", warnings)?;
    let emission = color_param(doc, model, "emission", warnings)?;
    if let Some(d) = diffuse {
        material.base_color = d;
    } else if model.tag_name().name() == "constant" {
        // A constant (unlit) material's colour is its emission.
        material.base_color = emission.unwrap_or(LinearRgba::BLACK);
    }

    // 3ds Max: self-illumination ("emission_level") makes the diffuse colour
    // emissive when no explicit emission is given.
    let emission_level = extra_float(doc, technique, "FCOLLADA", "emission_level")?.unwrap_or(0.0);
    material.emissive = match emission {
        Some(e) if model.tag_name().name() != "constant" => e,
        _ if emission_level > 0.0 => material.base_color * emission_level,
        _ => LinearRgba::BLACK,
    };
    material.emissive.a = 1.0;

    if let Some(specular) = color_param(doc, model, "specular", warnings)? {
        let level = extra_float(doc, technique, "FCOLLADA", "spec_level")?.unwrap_or(1.0);
        material.specular = LinearRgba {
            a: 1.0,
            ..specular * level
        };
    }
    if let Some(s) = float_param(doc, model, "shininess")? {
        material.shininess = if s <= NORMALIZED_SHININESS_MAX {
            s * NORMALIZED_SHININESS_SCALE
        } else {
            s
        };
    }

    // Opacity: <transparent> (A_ONE: colour alpha; RGB_ZERO: 1 - luminance)
    // scaled by <transparency>.
    let transparency = float_param(doc, model, "transparency")?.unwrap_or(1.0);
    if let Some(param) = child(model, "transparent")
        && let Some(color) = child(param, "color")
    {
        let [r, g, b, a] = fixed_numbers::<4>(doc, color)?;
        let opacity = match param.attribute("opaque").unwrap_or("A_ONE") {
            "RGB_ZERO" => 1.0 - (r * 0.2126 + g * 0.7152 + b * 0.0722) * transparency,
            _ => a * transparency,
        };
        material.base_color.a = opacity.clamp(0.0, 1.0);
    }

    if extra_float(doc, effect, "MAX3D", "double_sided")?.is_some_and(|v| v != 0.0) {
        material.double_sided = true;
    }
    Ok(material)
}

/// Converts a `<material>` (following its `<instance_effect>`).
pub(crate) fn parse_material(
    doc: &Document<'_>,
    material: Node<'_, '_>,
    warnings: &mut Vec<String>,
) -> Result<Material, ColladaError> {
    let name = material
        .attribute("name")
        .or_else(|| material.attribute("id"))
        .unwrap_or("");
    let effect = doc.by_uri(attr(
        doc,
        require(doc, material, "instance_effect")?,
        "url",
    )?)?;
    parse_effect(doc, effect, name, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba8;

    fn load(effect_body: &str) -> (Material, Vec<String>) {
        let xml = format!(
            r##"<COLLADA><library_materials><material id="m" name="Mat"><instance_effect url="#e"/></material></library_materials>
<library_effects><effect id="e">{effect_body}</effect></library_effects></COLLADA>"##
        );
        let doc = Document::parse(&xml).unwrap();
        let mut warnings = Vec::new();
        let m = parse_material(&doc, doc.by_uri("#m").unwrap(), &mut warnings).unwrap();
        (m, warnings)
    }

    fn srgb8(c: LinearRgba) -> [u8; 4] {
        c.to_srgb8().to_array()
    }

    #[test]
    fn phong_parameters() {
        let (m, w) = load(
            r#"<profile_COMMON><technique sid="common"><phong>
            <emission><color>0.1 0 0 1</color></emission>
            <diffuse><color>1 0.5 0 1</color></diffuse>
            <specular><color>1 1 1 1</color></specular>
            <shininess><float>20</float></shininess>
            <transparent opaque="A_ONE"><color>0 0 0 0.5</color></transparent>
            </phong></technique></profile_COMMON>"#,
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(m.name, "Mat");
        assert_eq!(srgb8(m.base_color), [255, 128, 0, 128]);
        assert_eq!(m.emissive.to_srgb8(), Rgba8::new(26, 0, 0, 255));
        assert_eq!(m.specular, LinearRgba::WHITE);
        assert_eq!(m.shininess, 20.0);
    }

    #[test]
    fn constant_uses_emission_as_base_colour() {
        let (m, _) = load(
            r#"<profile_COMMON><technique><constant><emission><color>0 1 0 1</color></emission></constant></technique></profile_COMMON>"#,
        );
        assert_eq!(srgb8(m.base_color), [0, 255, 0, 255]);
        assert_eq!(m.emissive, LinearRgba::BLACK);
    }

    #[test]
    fn textures_and_unknown_profiles_warn() {
        let (m, w) = load(
            r##"<profile_COMMON><technique><lambert><diffuse><texture texture="img" texcoord="uv"/></diffuse></lambert></technique></profile_COMMON>"##,
        );
        assert_eq!(m.base_color, LinearRgba::WHITE);
        assert_eq!(w.len(), 1);
        let (m, w) = load(r#"<profile_GLSL/>"#);
        assert_eq!(m.base_color, LinearRgba::WHITE);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn max_extras_set_self_illumination_specular_level_and_double_sided() {
        let (m, _) = load(
            r#"<profile_COMMON><technique><blinn>
            <diffuse><color>0.5 0.5 0.5 1</color></diffuse><specular><color>1 1 1 1</color></specular>
            <shininess><float>0.5</float></shininess></blinn>
            <extra><technique profile="FCOLLADA"><spec_level><float>0.25</float></spec_level><emission_level><float>1</float></emission_level></technique></extra>
            </technique></profile_COMMON><extra><technique profile="MAX3D"><double_sided>1</double_sided></technique></extra>"#,
        );
        assert_eq!(
            m.emissive,
            LinearRgba {
                a: 1.0,
                ..m.base_color
            },
            "fully self-illuminated"
        );
        assert_eq!(m.specular, LinearRgba::rgb(0.25, 0.25, 0.25));
        assert_eq!(m.shininess, 64.0, "normalized glossiness scaled");
        assert!(m.double_sided);
    }

    #[test]
    fn n64_logo_materials_match_readme_colours() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/n64_logo/n64_logo.dae"
        ))
        .unwrap();
        let doc = Document::parse(&text).unwrap();
        let mut warnings = Vec::new();
        // _1_ green, _2_ blue, _3_ red, _4_ yellow.
        let readme = [[6, 147, 48], [2, 34, 169], [255, 24, 19], [255, 192, 1]];
        for (i, expected) in readme.into_iter().enumerate() {
            let m = parse_material(
                &doc,
                doc.by_uri(&format!("#_{}_-_Default", i + 1)).unwrap(),
                &mut warnings,
            )
            .unwrap();
            let c = m.base_color.to_srgb8();
            assert_eq!(
                [c.r, c.g, c.b, c.a],
                [expected[0], expected[1], expected[2], 255],
                "{}",
                m.name
            );
            assert_eq!(
                m.emissive,
                LinearRgba {
                    a: 1.0,
                    ..m.base_color
                },
                "emission_level 1"
            );
            assert_eq!(m.specular, LinearRgba::BLACK, "spec_level 0");
            assert!(!m.double_sided);
        }
        assert!(warnings.is_empty(), "{warnings:?}");
    }
}
