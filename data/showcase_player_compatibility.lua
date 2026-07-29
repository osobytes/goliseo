-- GOLISEO showcase-only identity and species seams. These records preserve
-- the shipped showcase while PlayerData moves to the presentation/loadout model.

---@class ShowcasePlayerCompatibilityData
---@field player_id string
---@field planet string
---@field species string
---@field presentation_species string?
---@field trait string

---@type table<string, ShowcasePlayerCompatibilityData>
return {
    zyro_vex = {
        player_id = "zyro_vex",
        planet = "Kairon-9",
        species = "neutral",
        presentation_species = "voltari",
        trait = "comet_first_touch",
    },
    mika_olu = {
        player_id = "mika_olu",
        planet = "Vega Prime",
        species = "neutral",
        presentation_species = "myceloid",
        trait = "nebula_vision",
    },
    rok_tann = {
        player_id = "rok_tann",
        planet = "Titan Reach",
        species = "neutral",
        presentation_species = "terran",
        trait = "quantum_pass",
    },
    sela_dwin = {
        player_id = "sela_dwin",
        planet = "Andromeda Fringe",
        species = "neutral",
        presentation_species = "voltari",
        trait = "solar_flare_sprint",
    },
    brakka = {
        player_id = "brakka",
        planet = "Orion Belt",
        species = "neutral",
        presentation_species = "gravling",
        trait = "meteor_tackle",
    },
    veil_nyx = {
        player_id = "veil_nyx",
        planet = "Europa Deep",
        species = "neutral",
        presentation_species = "gravling",
        trait = "gravity_anchor",
    },
    ozzo = {
        player_id = "ozzo",
        planet = "Kairon-9",
        species = "neutral",
        presentation_species = "terran",
        trait = "zero_g_reflex",
    },
    tib_quell = {
        player_id = "tib_quell",
        planet = "Mars Colony",
        species = "neutral",
        presentation_species = "myceloid",
        trait = "comet_first_touch",
    },
    gax_oru = {
        player_id = "gax_oru",
        planet = "Orion Belt",
        species = "neutral",
        presentation_species = "gravling",
        trait = "gravity_anchor",
    },
    drell = {
        player_id = "drell",
        planet = "Orion Belt",
        species = "neutral",
        presentation_species = "gravling",
        trait = "meteor_tackle",
    },
    morv = {
        player_id = "morv",
        planet = "Ceres Outpost",
        species = "neutral",
        presentation_species = "terran",
        trait = "meteor_tackle",
    },
    krag = {
        player_id = "krag",
        planet = "Orion Belt",
        species = "neutral",
        presentation_species = "gravling",
        trait = "comet_first_touch",
    },
    tox_vren = {
        player_id = "tox_vren",
        planet = "Ceres Outpost",
        species = "neutral",
        presentation_species = "voltari",
        trait = "solar_flare_sprint",
    },
}
