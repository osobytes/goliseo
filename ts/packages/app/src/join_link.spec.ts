import { describe, expect, it } from "vitest";
import { joinUrl, roomCodeFromSearch, withoutRoomParam } from "./join_link.ts";

const VALID_CODE = "A3F9K2";

describe("roomCodeFromSearch", () => {
  it("is absent for a plain URL with no query string", () => {
    expect(roomCodeFromSearch("")).toBeUndefined();
  });

  it("is absent for an unrelated query parameter", () => {
    expect(roomCodeFromSearch("?debug=1")).toBeUndefined();
  });

  it("is absent for an empty room parameter", () => {
    expect(roomCodeFromSearch("?room=")).toBeUndefined();
  });

  it("is absent for junk that is not shaped like a room code (wrong length)", () => {
    expect(roomCodeFromSearch("?room=ABC")).toBeUndefined();
  });

  it("is absent for junk that is not shaped like a room code (bad alphabet)", () => {
    // "I", "L", "O", "U" are excluded from the Crockford base32 alphabet a
    // real room code is drawn from -- see `room_signaling.ts`'s own header.
    expect(roomCodeFromSearch("?room=AILOUZ")).toBeUndefined();
  });

  it("returns the code for a well-formed room parameter", () => {
    expect(roomCodeFromSearch(`?room=${VALID_CODE}`)).toBe(VALID_CODE);
  });

  it("returns the code for a well-formed room parameter alongside others", () => {
    expect(roomCodeFromSearch(`?debug=1&room=${VALID_CODE}&ice=relay`)).toBe(VALID_CODE);
  });

  it("normalizes a lowercase code to uppercase", () => {
    expect(roomCodeFromSearch(`?room=${VALID_CODE.toLowerCase()}`)).toBe(VALID_CODE);
  });

  it("normalizes a mixed-case code to uppercase", () => {
    expect(roomCodeFromSearch("?room=a3F9k2")).toBe(VALID_CODE);
  });
});

describe("joinUrl / roomCodeFromSearch round trip", () => {
  it("a code built into a join URL parses back out to the same code", () => {
    const origin = "https://goliseo.example";
    const url = joinUrl(origin, VALID_CODE);
    expect(url).toBe(`${origin}/?room=${VALID_CODE}`);
    const parsedSearch = new URL(url).search;
    expect(roomCodeFromSearch(parsedSearch)).toBe(VALID_CODE);
  });
});

describe("withoutRoomParam", () => {
  it("strips a bare ?room=<code> down to the plain path", () => {
    expect(withoutRoomParam(`https://goliseo.example/?room=${VALID_CODE}`)).toBe("/");
  });

  it("removes only the room parameter, keeping every other one", () => {
    expect(withoutRoomParam(`https://goliseo.example/?debug=1&room=${VALID_CODE}&ice=relay`)).toBe(
      "/?debug=1&ice=relay",
    );
  });

  it("preserves a path and hash beyond the origin", () => {
    expect(withoutRoomParam(`https://goliseo.example/app?room=${VALID_CODE}#lobby`)).toBe(
      "/app#lobby",
    );
  });

  it("is a no-op when there is no room parameter to strip", () => {
    expect(withoutRoomParam("https://goliseo.example/?debug=1")).toBe("/?debug=1");
  });
});
