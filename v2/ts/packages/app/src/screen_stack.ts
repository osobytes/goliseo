// Ported from game/screen_stack.lua.
//
// A minimal screen stack. The topmost screen receives update/event/draw.
// Screens are duck-typed: any of update/event/draw/teardown/applySettings
// may be absent.

export interface Screen<TEvent = unknown, TSettings = unknown> {
  update?(dt: number): void;
  event?(evt: TEvent): void;
  draw?(): void;
  teardown?(): void;
  applySettings?(settings: TSettings): void;
}

export class ScreenStack<TEvent = unknown, TSettings = unknown> {
  screens: Screen<TEvent, TSettings>[] = [];

  push(screen: Screen<TEvent, TSettings>): void {
    this.screens.push(screen);
  }

  private teardownScreen(screen: Screen<TEvent, TSettings> | undefined): void {
    if (screen?.teardown) {
      screen.teardown();
    }
  }

  replace(screen: Screen<TEvent, TSettings>): void {
    this.teardownScreen(this.screens[this.screens.length - 1]);
    this.screens[this.screens.length - 1] = screen;
  }

  clear(): void {
    for (let index = this.screens.length - 1; index >= 0; index -= 1) {
      this.teardownScreen(this.screens[index]);
    }
    this.screens = [];
  }

  pop(): Screen<TEvent, TSettings> | undefined {
    const screen = this.screens.pop();
    this.teardownScreen(screen);
    return screen;
  }

  current(): Screen<TEvent, TSettings> | undefined {
    return this.screens[this.screens.length - 1];
  }

  update(dt: number): void {
    const s = this.current();
    if (s?.update) {
      s.update(dt);
    }
  }

  event(evt: TEvent): void {
    const s = this.current();
    if (s?.event) {
      s.event(evt);
    }
  }

  draw(): void {
    const s = this.current();
    if (s?.draw) {
      s.draw();
    }
  }
}
