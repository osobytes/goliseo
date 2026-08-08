// Tiny shared shapes used across stadium.ts's helper modules.
//
// A `ShaderMaterial`/`onBeforeCompile` uniform is `{ value: T }` by
// three.js's own convention (`THREE.IUniform<T>`); `NumberUniform` names that
// shape for the scalar uniforms (`uTime`, `uExcitement`) `Stadium.update`/
// `Stadium.setExcitement` write into every frame. Builder modules hand back
// the SAME object they bound into a material's `uniforms` map (or into an
// `onBeforeCompile` closure) so `stadium.ts` can mutate `.value` directly
// without reaching back into three.js's own (GL-context-only) shader
// introspection -- see stadium_crowd.ts's file header for why this matters
// for headless testability.
export interface NumberUniform {
  value: number;
}
