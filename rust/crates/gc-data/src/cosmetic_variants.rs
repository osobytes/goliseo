//! Persistent, presentation-only variation. Values are semantic production ids,
//! not runtime material, mesh, or attachment objects.

/// A persistent cosmetic variation of a character presentation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CosmeticVariantData {
    /// Persistent identity, also the lookup key.
    pub id: &'static str,
    /// Character presentation this variant applies to.
    pub presentation_id: &'static str,
    /// Material variant id.
    pub material_variant_id: &'static str,
    /// Head variant id, if this presentation has swappable heads.
    pub head_variant_id: Option<&'static str>,
    /// Accessory id, if this variant carries one.
    pub accessory_id: Option<&'static str>,
}

/// Every authored cosmetic variant.
pub static ALL: &[CosmeticVariantData] = &[
    CosmeticVariantData {
        id: "rook_ember",
        presentation_id: "medieval_rook_emberguard",
        material_variant_id: "ember_bronze",
        head_variant_id: Some("closed_helm"),
        accessory_id: None,
    },
    CosmeticVariantData {
        id: "rook_steel",
        presentation_id: "medieval_rook_emberguard",
        material_variant_id: "moonlit_steel",
        head_variant_id: Some("open_helm"),
        accessory_id: None,
    },
    CosmeticVariantData {
        id: "bramble_moss",
        presentation_id: "medieval_bramble_quickstep",
        material_variant_id: "moss_green",
        head_variant_id: None,
        accessory_id: Some("short_cape"),
    },
    CosmeticVariantData {
        id: "bramble_berry",
        presentation_id: "medieval_bramble_quickstep",
        material_variant_id: "berry_red",
        head_variant_id: None,
        accessory_id: Some("belt_pouch"),
    },
    CosmeticVariantData {
        id: "nova_cyan",
        presentation_id: "scifi_nova_quell",
        material_variant_id: "ion_cyan",
        head_variant_id: Some("visor_clear"),
        accessory_id: None,
    },
    CosmeticVariantData {
        id: "nova_magenta",
        presentation_id: "scifi_nova_quell",
        material_variant_id: "nova_magenta",
        head_variant_id: Some("visor_dark"),
        accessory_id: None,
    },
    CosmeticVariantData {
        id: "axi_blue",
        presentation_id: "scifi_axi",
        material_variant_id: "signal_blue",
        head_variant_id: Some("sensor_round"),
        accessory_id: None,
    },
    CosmeticVariantData {
        id: "axi_orange",
        presentation_id: "scifi_axi",
        material_variant_id: "signal_orange",
        head_variant_id: Some("sensor_split"),
        accessory_id: None,
    },
    CosmeticVariantData {
        id: "moxie_sun",
        presentation_id: "toy_moxie_modular",
        material_variant_id: "sunburst",
        head_variant_id: Some("heroic"),
        accessory_id: None,
    },
    CosmeticVariantData {
        id: "moxie_ocean",
        presentation_id: "toy_moxie_modular",
        material_variant_id: "ocean",
        head_variant_id: Some("adventure"),
        accessory_id: None,
    },
    CosmeticVariantData {
        id: "tock_brass",
        presentation_id: "toy_tock",
        material_variant_id: "brass",
        head_variant_id: None,
        accessory_id: Some("square_key"),
    },
    CosmeticVariantData {
        id: "tock_cherry",
        presentation_id: "toy_tock",
        material_variant_id: "cherry",
        head_variant_id: None,
        accessory_id: Some("round_key"),
    },
];

/// Look up a cosmetic variant by id.
pub fn get(id: &str) -> Option<&'static CosmeticVariantData> {
    ALL.iter().find(|variant| variant.id == id)
}
