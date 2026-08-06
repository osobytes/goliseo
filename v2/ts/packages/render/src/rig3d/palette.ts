// One place for every colour in the slice, so the Roman read (oxblood cloth,
// bronze, bare steel, oiled leather) stays consistent across the parts files.

/** An RGB triple in [0, 1]. */
export type RGB = readonly [number, number, number];
/** An RGBA quad in [0, 1]. */
export type RGBA = readonly [number, number, number, number];

/** Named base colours shared across the slice's part builders. */
export interface Palette {
  readonly skin: RGB;
  readonly skin_dark: RGB;
  readonly tunic: RGB;
  readonly tunic_dark: RGB;
  readonly leather: RGB;
  readonly leather_dark: RGB;
  readonly steel: RGB;
  readonly steel_dark: RGB;
  readonly blade: RGB;
  readonly bronze: RGB;
  readonly bronze_dark: RGB;
  readonly gold: RGB;
  readonly bone: RGB;
  readonly eye: RGB;
  readonly crest: RGB;
  readonly sand: RGB;
  readonly sand_dark: RGB;
  readonly stone: RGB;
  readonly stone_dark: RGB;
  readonly shadow: RGBA;
  readonly gizmo_bone: RGBA;
  readonly gizmo_joint: RGBA;
}

/** One place for every colour in the slice. */
export const palette: Palette = {
  skin: [0.74, 0.55, 0.4],
  skin_dark: [0.62, 0.45, 0.32],

  tunic: [0.6, 0.13, 0.12], // oxblood wool
  tunic_dark: [0.44, 0.09, 0.09],
  leather: [0.36, 0.24, 0.15],
  leather_dark: [0.26, 0.17, 0.11],

  steel: [0.66, 0.69, 0.74], // lorica plate
  steel_dark: [0.44, 0.47, 0.53],
  blade: [0.8, 0.84, 0.9],
  bronze: [0.78, 0.56, 0.22],
  bronze_dark: [0.55, 0.38, 0.14],
  gold: [0.88, 0.72, 0.26],

  bone: [0.84, 0.78, 0.63], // ivory grip
  eye: [0.12, 0.11, 0.14],
  crest: [0.72, 0.14, 0.13], // helmet plume

  sand: [0.8, 0.69, 0.48],
  sand_dark: [0.7, 0.59, 0.39],
  stone: [0.62, 0.58, 0.51],
  stone_dark: [0.48, 0.45, 0.4],

  shadow: [0.1, 0.07, 0.05, 0.3],
  gizmo_bone: [1.0, 0.85, 0.2, 1.0],
  gizmo_joint: [0.2, 0.95, 1.0, 1.0],
};
