//! What the material picker lists: the rail's categories and each material's
//! one-line description, all derived from the material pack so a new material
//! shows up in the right places without UI code.

use pt_core::{Kind, MaterialTable, NONE};

/// How many materials the Recent category remembers.
pub const RECENT_LEN: usize = 12;

/// The kinds the rail lists, in rail order.
pub const KINDS: [Kind; 4] = [Kind::Powder, Kind::Liquid, Kind::Gas, Kind::Solid];

/// One entry in the picker's rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Category {
    /// The materials painted with most recently, newest first.
    Recent,
    /// Heat, cool, erase and the drawing toggles.
    Tools,
    Kind(Kind),
    /// Materials carrying this tag in the pack.
    Tag(String),
}

impl Category {
    /// The rail label: short enough for a 56-point-wide rail key.
    pub fn label(&self) -> String {
        match self {
            Category::Recent => "Recent".into(),
            Category::Tools => "Tools".into(),
            Category::Kind(Kind::Powder) => "Powder".into(),
            Category::Kind(Kind::Liquid) => "Liquid".into(),
            Category::Kind(Kind::Gas) => "Gas".into(),
            Category::Kind(Kind::Solid) => "Solid".into(),
            Category::Kind(Kind::Empty) => "Empty".into(),
            Category::Tag(tag) => {
                let words = tag.replace('-', " ");
                let mut chars = words.chars();
                chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
            }
        }
    }
}

/// The rail, top to bottom: Recent, Tools, every kind the pack uses, then
/// every tag the pack uses in the order it first appears.
pub fn categories(table: &MaterialTable) -> Vec<Category> {
    let mut out = vec![Category::Recent, Category::Tools];
    for kind in KINDS {
        if table.defs.iter().any(|d| d.kind == kind) {
            out.push(Category::Kind(kind));
        }
    }
    for def in &table.defs {
        for tag in &def.tags {
            let cat = Category::Tag(tag.clone());
            if !out.contains(&cat) {
                out.push(cat);
            }
        }
    }
    out
}

/// The materials listed under a kind or tag, in pack order. Recent and Tools
/// are not pack lists, so they come back empty.
pub fn members(table: &MaterialTable, category: &Category) -> Vec<u32> {
    let ids = 1..table.len() as u32;
    match category {
        Category::Kind(kind) => ids.filter(|&i| table.def(i).kind == *kind).collect(),
        Category::Tag(tag) => ids.filter(|&i| table.def(i).tags.contains(tag)).collect(),
        Category::Recent | Category::Tools => vec![],
    }
}

/// Moves `id` to the front of the recent list.
pub fn remember(recent: &mut Vec<u32>, id: u32) {
    recent.retain(|&r| r != id);
    recent.insert(0, id);
    recent.truncate(RECENT_LEN);
}

/// What the material does, in a few words, from its transitions: "boils to
/// steam at 100°", "burns at 250°". With two or more, the targets are left
/// out so the line still fits: "freezes at 0°, boils at 100°".
pub fn describe(table: &MaterialTable, id: u32) -> String {
    let def = table.def(id);
    let row = &table.gpu[id as usize];
    // (verb, target, temperature); cooling first, so it reads cold to hot.
    let mut changes: Vec<(&str, String, Option<f32>)> = vec![];
    if row.down_into != NONE {
        let to = table.def(row.down_into);
        changes.push((cool_verb(def.kind, to.kind), to.name.to_lowercase(), Some(row.down_temp)));
    }
    if let Some(h) = &def.heats_into {
        let to = table.def(table.expect_id(&h.into));
        changes.push((heat_verb(def.kind, to.kind), to.name.to_lowercase(), Some(h.at)));
    }
    if let Some(b) = &def.burns {
        changes.push(("burns", String::new(), Some(b.at)));
    }
    if let Some(d) = &def.decays {
        let to = table.def(table.expect_id(&d.into));
        if to.kind == Kind::Empty {
            changes.push(("fades away", String::new(), None));
        } else {
            changes.push(("decays", to.name.to_lowercase(), None));
        }
    }
    if changes.is_empty() {
        return match def.kind {
            Kind::Solid => "holds its shape",
            Kind::Powder => "piles up",
            Kind::Liquid => "flows",
            Kind::Gas => "drifts",
            Kind::Empty => "",
        }
        .into();
    }
    let with_target = changes.len() == 1;
    changes
        .iter()
        .map(|(verb, to, at)| {
            let mut s = verb.to_string();
            if with_target && !to.is_empty() {
                s += &format!(" to {to}");
            }
            if let Some(t) = at {
                s += &format!(" at {}", degrees(*t));
            }
            s
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Density for the row's right edge: "ρ 1600", "ρ 0.6".
pub fn density_label(table: &MaterialTable, id: u32) -> String {
    let d = table.def(id).density;
    if d >= 10.0 {
        format!("ρ {d:.0}")
    } else {
        format!("ρ {d:.1}")
    }
}

fn degrees(t: f32) -> String {
    if t.fract() == 0.0 {
        format!("{t:.0}°")
    } else {
        format!("{t:.1}°")
    }
}

fn heat_verb(from: Kind, to: Kind) -> &'static str {
    match (from, to) {
        (Kind::Solid | Kind::Powder, Kind::Liquid) => "melts",
        (Kind::Liquid, Kind::Gas) => "boils",
        (Kind::Solid | Kind::Powder, Kind::Gas) => "sublimes",
        _ => "turns",
    }
}

fn cool_verb(from: Kind, to: Kind) -> &'static str {
    match (from, to) {
        (Kind::Liquid, Kind::Solid | Kind::Powder) => "freezes",
        (Kind::Gas, Kind::Liquid) => "condenses",
        (Kind::Gas, Kind::Solid | Kind::Powder) => "deposits",
        _ => "turns",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn describe_core(name: &str) -> String {
        let t = MaterialTable::core();
        describe(&t, t.expect_id(name))
    }

    #[test]
    fn descriptions_come_from_the_transitions() {
        assert_eq!(describe_core("ice"), "melts to water at 0°");
        assert_eq!(describe_core("water"), "freezes at 0°, boils at 100°");
        assert_eq!(describe_core("steam"), "condenses to water at 100°");
        assert_eq!(describe_core("stone"), "melts to lava at 1000°");
        assert_eq!(describe_core("lava"), "freezes to stone at 1000°");
        assert_eq!(describe_core("wood"), "burns at 250°");
        assert_eq!(describe_core("fire"), "fades away");
        assert_eq!(describe_core("sand"), "piles up");
        assert_eq!(describe_core("wall"), "holds its shape");
    }

    #[test]
    fn density_labels() {
        let t = MaterialTable::core();
        assert_eq!(density_label(&t, t.expect_id("sand")), "ρ 1600");
        assert_eq!(density_label(&t, t.expect_id("steam")), "ρ 0.6");
    }

    #[test]
    fn core_rail_has_recent_tools_and_the_four_kinds() {
        let t = MaterialTable::core();
        assert_eq!(
            categories(&t),
            [
                Category::Recent,
                Category::Tools,
                Category::Kind(Kind::Powder),
                Category::Kind(Kind::Liquid),
                Category::Kind(Kind::Gas),
                Category::Kind(Kind::Solid),
            ]
        );
        let liquids: Vec<_> = members(&t, &Category::Kind(Kind::Liquid))
            .into_iter()
            .map(|i| t.def(i).id.as_str())
            .collect();
        assert_eq!(liquids, ["water", "lava", "oil"]);
    }

    #[test]
    fn tags_add_rail_entries_in_pack_order() {
        let mut src = pt_core::materials::CORE_PACK.to_string();
        src = src.replace("id = \"oil\"", "id = \"oil\"\ntags = [\"flammable\"]");
        src = src.replace("id = \"wood\"", "id = \"wood\"\ntags = [\"flammable\", \"plant-life\"]");
        let t = MaterialTable::from_toml(&src).unwrap();
        let cats = categories(&t);
        assert_eq!(cats[6..], [Category::Tag("flammable".into()), Category::Tag("plant-life".into())]);
        assert_eq!(cats[7].label(), "Plant life");
        assert_eq!(members(&t, &cats[6]), [t.expect_id("wood"), t.expect_id("oil")]);
    }

    #[test]
    fn remember_moves_to_front_and_caps() {
        let mut recent = vec![1, 2, 3];
        remember(&mut recent, 3);
        assert_eq!(recent, [3, 1, 2]);
        for id in 10..30 {
            remember(&mut recent, id);
        }
        assert_eq!(recent.len(), RECENT_LEN);
        assert_eq!(recent[0], 29);
    }
}
