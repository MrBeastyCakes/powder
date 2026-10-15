//! Materials as data.
//!
//! A material pack is a TOML file (see `materials/core.toml`). Loading it
//! validates every definition and compiles the pack into a fixed-size table of
//! [`GpuMaterial`] rows that both simulations read from a uniform buffer.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use serde::Deserialize;

/// The schema version this build reads. Packs with another version are refused
/// until a migration exists for them.
pub const SCHEMA_VERSION: u32 = 1;

/// Size of the material table on the GPU. Cells store the material id in
/// 8 bits, but the table is kept at 128 rows so it fits the smallest uniform
/// buffer limit phones report (16 KiB).
pub const MAX_MATERIALS: usize = 128;

/// Marks "no target material" in the GPU table.
pub const NONE: u32 = u32::MAX;

/// The built-in pack, embedded so the app runs without any files on disk.
pub const CORE_PACK: &str = include_str!("../../../materials/core.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Empty = 0,
    Solid = 1,
    Powder = 2,
    Liquid = 3,
    Gas = 4,
}

impl Kind {
    pub fn moves(self) -> bool {
        !matches!(self, Kind::Solid)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeatsInto {
    pub into: String,
    pub at: f32,
    pub latent: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Burns {
    pub into: String,
    pub at: f32,
    pub heat: f32,
    pub chance: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decays {
    pub into: String,
    pub chance: f32,
}

fn room_temp() -> f32 {
    20.0
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialDef {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// Extra categories the material is listed under besides its kind, like
    /// `"explosive"`: lower case letters, digits and `-`.
    #[serde(default)]
    pub tags: Vec<String>,
    /// `#rrggbb` in sRGB.
    pub color: String,
    pub density: f32,
    pub heat_capacity: f32,
    pub conductivity: f32,
    #[serde(default = "room_temp")]
    pub spawn_temp: f32,
    /// Powders: chance per tick of not sliding sideways off a pile.
    #[serde(default)]
    pub friction: f32,
    /// Liquids and gases: chance per tick of flowing sideways.
    #[serde(default)]
    pub spread: f32,
    /// How much the material lights itself, 0 to 1.
    #[serde(default)]
    pub glow: f32,
    pub heats_into: Option<HeatsInto>,
    pub burns: Option<Burns>,
    pub decays: Option<Decays>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackFile {
    schema_version: u32,
    pack: String,
    #[serde(rename = "material")]
    materials: Vec<MaterialDef>,
}

/// One row of the material table, laid out for a WGSL uniform array
/// (80 bytes, a multiple of 16). Must match `Material` in `common.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GpuMaterial {
    pub kind: u32,
    /// sRGB color packed as `r | g << 8 | b << 16 | a << 24`.
    pub color: u32,
    pub density: f32,
    pub heat_capacity: f32,

    pub conductivity: f32,
    pub friction: f32,
    pub spread: f32,
    pub glow: f32,

    pub up_temp: f32,
    pub up_into: u32,
    pub up_latent: f32,
    pub down_temp: f32,

    pub down_into: u32,
    pub down_latent: f32,
    pub burn_temp: f32,
    pub burn_into: u32,

    pub burn_heat: f32,
    pub burn_chance: f32,
    pub decay_into: u32,
    pub decay_chance: f32,
}

const _: () = assert!(std::mem::size_of::<GpuMaterial>() == 80);

impl GpuMaterial {
    /// A row nothing references; fills the unused tail of the table.
    pub const UNUSED: GpuMaterial = GpuMaterial {
        kind: Kind::Solid as u32,
        color: 0xff_ff_00_ff,
        density: 1.0,
        heat_capacity: 1.0,
        conductivity: 0.0,
        friction: 0.0,
        spread: 0.0,
        glow: 0.0,
        up_temp: f32::MAX,
        up_into: NONE,
        up_latent: 0.0,
        down_temp: f32::MIN,
        down_into: NONE,
        down_latent: 0.0,
        burn_temp: f32::MAX,
        burn_into: NONE,
        burn_heat: 0.0,
        burn_chance: 0.0,
        decay_into: NONE,
        decay_chance: 0.0,
    };
}

#[derive(Debug, thiserror::Error)]
pub enum MaterialError {
    #[error("could not parse material pack: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("pack uses schema version {found}, this build reads version {expected}")]
    Schema { found: u32, expected: u32 },
    #[error("the pack has no materials")]
    Empty,
    #[error("the pack has {0} materials, the limit is {MAX_MATERIALS}")]
    TooMany(usize),
    #[error("material `{0}` is defined twice")]
    Duplicate(String),
    #[error("the first material must have kind = \"empty\", found `{0}`")]
    FirstNotEmpty(String),
    #[error("material `{0}`: only the first material may be empty")]
    ExtraEmpty(String),
    #[error("material `{material}`: {field} refers to unknown material `{target}`")]
    UnknownTarget {
        material: String,
        field: &'static str,
        target: String,
    },
    #[error("material `{material}`: {problem}")]
    Invalid { material: String, problem: String },
}

fn invalid(material: &str, problem: impl Into<String>) -> MaterialError {
    MaterialError::Invalid {
        material: material.to_string(),
        problem: problem.into(),
    }
}

/// A validated, compiled material pack.
#[derive(Debug, Clone)]
pub struct MaterialTable {
    pub pack: String,
    pub defs: Vec<MaterialDef>,
    pub gpu: Vec<GpuMaterial>,
    index: HashMap<String, u32>,
}

impl MaterialTable {
    /// The built-in pack. It is part of the build, so failing to load it is a bug.
    pub fn core() -> MaterialTable {
        MaterialTable::from_toml(CORE_PACK).expect("the built-in material pack is valid")
    }

    pub fn from_toml(src: &str) -> Result<MaterialTable, MaterialError> {
        let file: PackFile = toml::from_str(src)?;
        if file.schema_version != SCHEMA_VERSION {
            return Err(MaterialError::Schema {
                found: file.schema_version,
                expected: SCHEMA_VERSION,
            });
        }
        MaterialTable::compile(file.pack, file.materials)
    }

    pub fn compile(pack: String, defs: Vec<MaterialDef>) -> Result<MaterialTable, MaterialError> {
        if defs.is_empty() {
            return Err(MaterialError::Empty);
        }
        if defs.len() > MAX_MATERIALS {
            return Err(MaterialError::TooMany(defs.len()));
        }

        let mut index = HashMap::new();
        for (i, def) in defs.iter().enumerate() {
            if index.insert(def.id.clone(), i as u32).is_some() {
                return Err(MaterialError::Duplicate(def.id.clone()));
            }
        }
        if defs[0].kind != Kind::Empty {
            return Err(MaterialError::FirstNotEmpty(defs[0].id.clone()));
        }

        let lookup = |material: &str, field: &'static str, target: &str| {
            index.get(target).copied().ok_or_else(|| MaterialError::UnknownTarget {
                material: material.to_string(),
                field,
                target: target.to_string(),
            })
        };

        let mut gpu = Vec::with_capacity(MAX_MATERIALS);
        for (i, def) in defs.iter().enumerate() {
            validate(def, i)?;
            let mut row = GpuMaterial {
                kind: def.kind as u32,
                color: parse_color(&def.color).ok_or_else(|| invalid(&def.id, "color must be #rrggbb"))?,
                density: def.density,
                heat_capacity: def.heat_capacity,
                conductivity: def.conductivity,
                friction: def.friction,
                spread: def.spread,
                glow: def.glow,
                ..GpuMaterial::UNUSED
            };
            if let Some(h) = &def.heats_into {
                row.up_into = lookup(&def.id, "heats_into", &h.into)?;
                row.up_temp = h.at;
                row.up_latent = h.latent;
            }
            if let Some(b) = &def.burns {
                row.burn_into = lookup(&def.id, "burns", &b.into)?;
                row.burn_temp = b.at;
                row.burn_heat = b.heat;
                row.burn_chance = b.chance;
            }
            if let Some(d) = &def.decays {
                row.decay_into = lookup(&def.id, "decays", &d.into)?;
                row.decay_chance = d.chance;
            }
            gpu.push(row);
        }

        // Derive the reverse of every `heats_into`: water heats into steam at
        // 100, so steam cools back into water at 100 and releases the latent heat.
        for (i, def) in defs.iter().enumerate() {
            let Some(h) = &def.heats_into else { continue };
            let target = index[&h.into] as usize;
            if target == i {
                return Err(invalid(&def.id, "heats_into cannot target itself"));
            }
            if gpu[target].down_into != NONE {
                let other = &defs[gpu[target].down_into as usize].id;
                return Err(invalid(
                    &defs[target].id,
                    format!(
                        "both `{other}` and `{}` heat into it; a material can only cool back into one",
                        def.id
                    ),
                ));
            }
            if gpu[target].up_temp <= h.at {
                return Err(invalid(
                    &defs[target].id,
                    format!(
                        "it heats into something at {} but forms from `{}` at {}; the first must be higher",
                        gpu[target].up_temp, def.id, h.at
                    ),
                ));
            }
            gpu[target].down_into = i as u32;
            gpu[target].down_temp = h.at;
            gpu[target].down_latent = h.latent;
        }

        gpu.resize(MAX_MATERIALS, GpuMaterial::UNUSED);
        Ok(MaterialTable { pack, defs, gpu, index })
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    pub fn id(&self, name: &str) -> Option<u32> {
        self.index.get(name).copied()
    }

    /// Like [`id`](Self::id), for materials the caller knows exist.
    pub fn expect_id(&self, name: &str) -> u32 {
        self.id(name)
            .unwrap_or_else(|| panic!("material `{name}` is not in pack `{}`", self.pack))
    }

    pub fn def(&self, id: u32) -> &MaterialDef {
        &self.defs[id as usize]
    }

    /// The table as bytes, ready for a uniform buffer.
    pub fn gpu_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.gpu)
    }

    /// The material's color as `[r, g, b]` bytes, for UI swatches.
    pub fn rgb(&self, id: u32) -> [u8; 3] {
        let c = self.gpu[id as usize].color;
        [c as u8, (c >> 8) as u8, (c >> 16) as u8]
    }
}

fn validate(def: &MaterialDef, index: usize) -> Result<(), MaterialError> {
    if def.kind == Kind::Empty && index != 0 {
        return Err(MaterialError::ExtraEmpty(def.id.clone()));
    }
    let positive = [("density", def.density), ("heat_capacity", def.heat_capacity)];
    for (field, v) in positive {
        if !(v.is_finite() && v > 0.0) {
            return Err(invalid(&def.id, format!("{field} must be a positive number")));
        }
    }
    for (i, tag) in def.tags.iter().enumerate() {
        let well_formed =
            !tag.is_empty() && tag.len() <= 24 && tag.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !well_formed {
            return Err(invalid(
                &def.id,
                format!("tag `{tag}` must be 1 to 24 lower case letters, digits or `-`"),
            ));
        }
        if def.tags[..i].contains(tag) {
            return Err(invalid(&def.id, format!("tag `{tag}` is listed twice")));
        }
    }
    if !(def.conductivity.is_finite() && def.conductivity >= 0.0) {
        return Err(invalid(&def.id, "conductivity must be zero or positive"));
    }
    let unit = [("friction", def.friction), ("spread", def.spread), ("glow", def.glow)];
    for (field, v) in unit {
        if !(0.0..=1.0).contains(&v) {
            return Err(invalid(&def.id, format!("{field} must be between 0 and 1")));
        }
    }
    if !(-273.15..=10_000.0).contains(&def.spawn_temp) {
        return Err(invalid(&def.id, "spawn_temp must be between -273.15 and 10000"));
    }
    if let Some(h) = &def.heats_into {
        if !(h.latent.is_finite() && h.latent >= 0.0) {
            return Err(invalid(&def.id, "heats_into.latent must be zero or positive"));
        }
    }
    if let Some(b) = &def.burns {
        if !(0.0..=1.0).contains(&b.chance) {
            return Err(invalid(&def.id, "burns.chance must be between 0 and 1"));
        }
    }
    if let Some(d) = &def.decays {
        if !(0.0..=1.0).contains(&d.chance) {
            return Err(invalid(&def.id, "decays.chance must be between 0 and 1"));
        }
    }
    Ok(())
}

fn parse_color(s: &str) -> Option<u32> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    let (r, g, b) = ((v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff);
    Some(r | g << 8 | b << 16 | 0xff << 24)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r##"
        schema_version = 1
        pack = "test"
        [[material]]
        id = "air"
        name = "Air"
        kind = "empty"
        color = "#000000"
        density = 1.2
        heat_capacity = 0.05
        conductivity = 0.02
    "##;

    fn with(extra: &str) -> String {
        format!("{MINIMAL}\n{extra}")
    }

    #[test]
    fn core_pack_loads() {
        let t = MaterialTable::core();
        assert_eq!(t.def(0).kind, Kind::Empty);
        assert!(t.len() >= 11, "core pack has air plus at least 10 materials");
        assert_eq!(t.gpu.len(), MAX_MATERIALS);
        assert_eq!(t.gpu_bytes().len(), MAX_MATERIALS * 80);
    }

    #[test]
    fn reverse_transitions_are_derived() {
        let t = MaterialTable::core();
        let (ice, water, steam) = (t.expect_id("ice"), t.expect_id("water"), t.expect_id("steam"));
        let w = t.gpu[water as usize];
        assert_eq!(w.up_into, steam);
        assert_eq!(w.up_temp, 100.0);
        assert_eq!(w.down_into, ice);
        assert_eq!(w.down_temp, 0.0);
        assert_eq!(w.down_latent, t.gpu[ice as usize].up_latent);
        assert_eq!(t.gpu[steam as usize].down_into, water);
        assert_eq!(t.gpu[steam as usize].up_into, NONE);
    }

    #[test]
    fn color_is_packed_rgba() {
        assert_eq!(parse_color("#112233"), Some(0xff_33_22_11));
        assert_eq!(parse_color("112233"), None);
        assert_eq!(parse_color("#12345"), None);
    }

    #[test]
    fn rejects_unknown_target() {
        let src = with(
            r##"
            [[material]]
            id = "a"
            name = "A"
            kind = "solid"
            color = "#ffffff"
            density = 1
            heat_capacity = 1
            conductivity = 1
            heats_into = { into = "nope", at = 10, latent = 1 }
        "##,
        );
        assert!(matches!(MaterialTable::from_toml(&src), Err(MaterialError::UnknownTarget { .. })));
    }

    #[test]
    fn rejects_two_materials_heating_into_one() {
        let mat = |id: &str, extra: &str| {
            format!(
                "[[material]]\nid = \"{id}\"\nname = \"{id}\"\nkind = \"solid\"\ncolor = \"#ffffff\"\ndensity = 1\nheat_capacity = 1\nconductivity = 1\n{extra}\n"
            )
        };
        let src = with(&format!(
            "{}{}{}",
            mat("a", "heats_into = { into = \"c\", at = 10, latent = 1 }"),
            mat("b", "heats_into = { into = \"c\", at = 20, latent = 1 }"),
            mat("c", "")
        ));
        assert!(matches!(MaterialTable::from_toml(&src), Err(MaterialError::Invalid { .. })));
    }

    #[test]
    fn rejects_wrong_schema_and_unknown_fields() {
        let src = MINIMAL.replace("schema_version = 1", "schema_version = 2");
        assert!(matches!(MaterialTable::from_toml(&src), Err(MaterialError::Schema { .. })));
        let src = MINIMAL.replace("conductivity = 0.02", "conductivity = 0.02\nviscosity = 3");
        assert!(matches!(MaterialTable::from_toml(&src), Err(MaterialError::Parse(_))));
    }

    #[test]
    fn tags_are_optional_and_checked() {
        let mat = |tags: &str| {
            with(&format!(
                "[[material]]\nid = \"a\"\nname = \"A\"\nkind = \"powder\"\ncolor = \"#ffffff\"\ndensity = 1\nheat_capacity = 1\nconductivity = 1\n{tags}\n"
            ))
        };
        assert!(MaterialTable::from_toml(&mat("")).unwrap().def(1).tags.is_empty());
        let t = MaterialTable::from_toml(&mat("tags = [\"explosive\", \"hot-2\"]")).unwrap();
        assert_eq!(t.def(1).tags, ["explosive", "hot-2"]);
        for bad in ["[\"\"]", "[\"Explosive\"]", "[\"a b\"]", "[\"x\", \"x\"]"] {
            let src = mat(&format!("tags = {bad}"));
            assert!(
                matches!(MaterialTable::from_toml(&src), Err(MaterialError::Invalid { .. })),
                "{bad}"
            );
        }
    }

    #[test]
    fn rejects_empty_first_rule_violations() {
        let src = MINIMAL.replace("kind = \"empty\"", "kind = \"solid\"");
        assert!(matches!(MaterialTable::from_toml(&src), Err(MaterialError::FirstNotEmpty(_))));
    }
}
