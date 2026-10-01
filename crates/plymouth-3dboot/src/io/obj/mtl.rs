// SPDX-License-Identifier: GPL-3.0-or-later
//! Wavefront MTL material libraries.

use super::parse::{ObjError, ObjErrorKind, Warning};
use crate::color::LinearRgba;
use crate::scene::Material;

/// A material as written in an MTL file (values unconverted).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MtlMaterial {
    /// `newmtl` name.
    pub name: String,
    /// `Ka` ambient colour.
    pub ambient: Option<[f32; 3]>,
    /// `Kd` diffuse colour.
    pub diffuse: Option<[f32; 3]>,
    /// `Ks` specular colour.
    pub specular: Option<[f32; 3]>,
    /// `Ke` emissive colour.
    pub emissive: Option<[f32; 3]>,
    /// `Ns` specular exponent.
    pub shininess: Option<f32>,
    /// `d` opacity (1 = opaque).
    pub dissolve: Option<f32>,
    /// `Tr` transparency (0 = opaque); only used when `d` is absent.
    pub transparency: Option<f32>,
    /// `illum` illumination model.
    pub illum: Option<u32>,
}

impl MtlMaterial {
    /// Converts to a [`Material`]. Colours are treated as sRGB-encoded (see
    /// `docs/conventions.md`) and decoded to linear. Opacity comes from `d`,
    /// or from `1 - Tr` if `d` is absent (exporters disagree on `Tr`, so `d`
    /// wins when both are present).
    #[must_use]
    pub fn to_material(&self) -> Material {
        let rgb = |c: Option<[f32; 3]>, default: f32| {
            let [r, g, b] = c.unwrap_or([default; 3]);
            LinearRgba::from_srgb(r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), 1.0)
        };
        let alpha = self
            .dissolve
            .or(self.transparency.map(|t| 1.0 - t))
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        Material {
            name: self.name.clone(),
            base_color: LinearRgba {
                a: alpha,
                ..rgb(self.diffuse, 0.8)
            },
            emissive: rgb(self.emissive, 0.0),
            specular: rgb(self.specular, 0.0),
            shininess: self.shininess.unwrap_or(0.0).max(0.0),
            double_sided: false,
        }
    }
}

/// The contents of an MTL file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MtlData {
    /// Materials in file order.
    pub materials: Vec<MtlMaterial>,
    /// Ignored directives (e.g. texture maps, `Ni`, `Tf`).
    pub warnings: Vec<Warning>,
}

fn floats<const N: usize>(
    tokens: &mut std::str::SplitWhitespace<'_>,
    what: &'static str,
) -> Result<[f32; N], ObjErrorKind> {
    let mut out = [0.0; N];
    for v in &mut out {
        let t = tokens.next().ok_or(ObjErrorKind::MissingValue(what))?;
        *v = t
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| ObjErrorKind::InvalidNumber(t.to_owned()))?;
    }
    Ok(out)
}

/// Parses MTL source text (LF or CRLF line endings).
///
/// # Errors
///
/// Returns an [`ObjError`] for malformed numbers, missing values, or
/// properties before the first `newmtl`.
pub fn parse_mtl(source: &str) -> Result<MtlData, ObjError> {
    let mut data = MtlData::default();
    for (i, raw) in source.lines().enumerate() {
        let line = i + 1;
        let err = |kind| ObjError { line, kind };
        let text = raw.split('#').next().unwrap_or("").trim();
        let mut tokens = text.split_whitespace();
        let Some(keyword) = tokens.next() else {
            continue;
        };
        if keyword == "newmtl" {
            let name = text["newmtl".len()..].trim();
            if name.is_empty() {
                return Err(err(ObjErrorKind::MissingValue("newmtl")));
            }
            data.materials.push(MtlMaterial {
                name: name.to_owned(),
                ..MtlMaterial::default()
            });
            continue;
        }
        let Some(current) = data.materials.last_mut() else {
            return Err(err(ObjErrorKind::MissingValue("newmtl")));
        };
        match keyword {
            "Ka" => current.ambient = Some(floats(&mut tokens, "Ka").map_err(err)?),
            "Kd" => current.diffuse = Some(floats(&mut tokens, "Kd").map_err(err)?),
            "Ks" => current.specular = Some(floats(&mut tokens, "Ks").map_err(err)?),
            "Ke" => current.emissive = Some(floats(&mut tokens, "Ke").map_err(err)?),
            "Ns" => current.shininess = Some(floats::<1>(&mut tokens, "Ns").map_err(err)?[0]),
            "d" => current.dissolve = Some(floats::<1>(&mut tokens, "d").map_err(err)?[0]),
            "Tr" => current.transparency = Some(floats::<1>(&mut tokens, "Tr").map_err(err)?[0]),
            "illum" => {
                let t = tokens
                    .next()
                    .ok_or(err(ObjErrorKind::MissingValue("illum")))?;
                current.illum = Some(
                    t.parse()
                        .map_err(|_| err(ObjErrorKind::InvalidNumber(t.to_owned())))?,
                );
            }
            other => data.warnings.push(Warning {
                line,
                message: format!("ignored directive {other:?}"),
            }),
        }
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba8;

    #[test]
    fn parses_properties_per_material() {
        let d = parse_mtl("newmtl a\nKd 1 0.5 0\nNs 20\nd 0.5\nillum 2\n\nnewmtl b b\nKe 0.1 0.2 0.3\nTr 0.25\nmap_Kd x.png\n").unwrap();
        assert_eq!(d.materials.len(), 2);
        let (a, b) = (&d.materials[0], &d.materials[1]);
        assert_eq!(
            (a.name.as_str(), a.diffuse, a.shininess, a.dissolve, a.illum),
            ("a", Some([1.0, 0.5, 0.0]), Some(20.0), Some(0.5), Some(2))
        );
        assert_eq!(
            (b.name.as_str(), b.emissive, b.transparency),
            ("b b", Some([0.1, 0.2, 0.3]), Some(0.25))
        );
        assert_eq!(d.warnings.len(), 1);
        assert_eq!(d.warnings[0].line, 10);
    }

    #[test]
    fn conversion_decodes_srgb_and_opacity() {
        let m = MtlMaterial {
            name: "m".into(),
            diffuse: Some([1.0, 0.5, 0.0]),
            dissolve: Some(0.5),
            transparency: Some(1.0),
            shininess: Some(8.0),
            ..MtlMaterial::default()
        };
        let mat = m.to_material();
        assert_eq!(mat.base_color.to_srgb8(), Rgba8::new(255, 128, 0, 128));
        assert_eq!(mat.shininess, 8.0);
        // `d` wins over `Tr`; `Tr` alone is transparency.
        let tr_only = MtlMaterial {
            transparency: Some(0.25),
            ..MtlMaterial::default()
        }
        .to_material();
        assert_eq!(tr_only.base_color.a, 0.75);
        // Defaults: grey diffuse, no emission.
        let none = MtlMaterial::default().to_material();
        assert_eq!(none.emissive, LinearRgba::rgb(0.0, 0.0, 0.0));
        assert_eq!(none.base_color.a, 1.0);
    }

    #[test]
    fn errors() {
        assert_eq!(
            parse_mtl("Kd 1 1 1\n"),
            Err(ObjError {
                line: 1,
                kind: ObjErrorKind::MissingValue("newmtl")
            })
        );
        assert_eq!(
            parse_mtl("newmtl a\nKd 1 x 1\n"),
            Err(ObjError {
                line: 2,
                kind: ObjErrorKind::InvalidNumber("x".into())
            })
        );
        assert_eq!(
            parse_mtl("newmtl a\nKd 1 1\n"),
            Err(ObjError {
                line: 2,
                kind: ObjErrorKind::MissingValue("Kd")
            })
        );
    }

    #[test]
    fn n64_logo_materials_match_readme_colours() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/n64_logo/n64_logo.mtl"
        );
        let d = parse_mtl(&std::fs::read_to_string(path).unwrap()).unwrap();
        // Readme: green, blue, red, yellow (sRGB 0-255).
        let readme = [[6, 147, 48], [2, 34, 169], [255, 24, 19], [255, 192, 1]];
        assert_eq!(d.materials.len(), 4);
        for (m, expected) in d.materials.iter().zip(readme) {
            let kd = m.diffuse.unwrap();
            for (v, e) in kd.iter().zip(expected) {
                assert!(
                    (v * 255.0 - f32::from(e)).abs() <= 1.0,
                    "{}: {kd:?} vs {expected:?}",
                    m.name
                );
            }
            assert_eq!(m.emissive, m.diffuse, "Ke = Kd in this file");
            let srgb = m.to_material().base_color.to_srgb8();
            assert_eq!([srgb.r, srgb.g, srgb.b], expected, "{}", m.name);
            assert_eq!(srgb.a, 255, "d 1.0 wins over Tr 1.0");
        }
    }
}
