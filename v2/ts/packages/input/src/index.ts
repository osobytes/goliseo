// Input capture and binding (browser side of the boundary).
export { bindings, TRIGGER_THRESHOLD } from "./bindings.ts";
export type {
  ControlBinding,
  ControlId,
  ControlReferenceRow,
  ControlSection,
  GamepadState,
  KeyboardState,
} from "./bindings.ts";
export { actions } from "./actions.ts";
export type { ActionEvent, ActionName, InputSource } from "./actions.ts";
export { controller } from "./controller.ts";
export type {
  ClickEvent,
  ControllerInputEvent,
  KeyEvent,
  RawGamepadEvent,
  ViewportMapper,
  ViewportTransform,
} from "./controller.ts";
export { invariant } from "./assert.ts";

// The browser capture surface. Namespaced rather than flattened: `VERSION`,
// `translateCode` and the several `*_MAP` tables are generic enough at a
// package root to collide with, or be mistaken for, something else.
export * as inputSample from "./input_sample.ts";
export * as captureKeyboard from "./capture_keyboard.ts";
export * as captureGamepad from "./capture_gamepad.ts";
export * as captureFrame from "./capture_frame.ts";
export type * as inputSampleTypes from "./input_sample.ts";
export type * as captureKeyboardTypes from "./capture_keyboard.ts";
export type * as captureGamepadTypes from "./capture_gamepad.ts";
export type * as captureFrameTypes from "./capture_frame.ts";
