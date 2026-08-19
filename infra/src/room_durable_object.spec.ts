import { describe, expect, it } from "vitest";

import { closeCodeForAdmissionFailure } from "./room_durable_object.ts";

// `RoomDurableObject`'s own fetch()/webSocketMessage()/etc. need a real
// Workers runtime (WebSocketPair, DurableObjectState, the SQLite storage
// API) that does not exist under plain Node -- vitest.config.ts's own doc
// disclaims that half as `wrangler dev` territory, not a headless-gate
// target. `closeCodeForAdmissionFailure` is the one piece of this file's
// admission-failure behavior that is plain data, so it is exported and
// tested directly: one assertion per in-band reason `claimHost`/`joinGuest`
// (room_state.ts) can actually produce, per this module's own doc,
// "Admission failures".
describe("closeCodeForAdmissionFailure", () => {
  it("maps every admission-failure reason to a distinct 4000-4999 close code", () => {
    const reasons = [
      "room_not_found",
      "room_full",
      "room_expired",
      "room_closed",
      "host_already_claimed",
      "already_joined",
    ];
    const codes = reasons.map((reason) => closeCodeForAdmissionFailure(reason));
    for (const code of codes) {
      expect(code).toBeGreaterThanOrEqual(4000);
      expect(code).toBeLessThanOrEqual(4999);
    }
    // Distinct -- a Set collapses duplicates, so its size only matches the
    // reason count if every code differs.
    expect(new Set(codes).size).toBe(reasons.length);
  });

  it("falls back to a fixed default for an unrecognized reason", () => {
    expect(closeCodeForAdmissionFailure("something_new")).toBe(
      closeCodeForAdmissionFailure("another_unknown_reason"),
    );
    expect(closeCodeForAdmissionFailure("something_new")).toBeGreaterThanOrEqual(4000);
  });
});
