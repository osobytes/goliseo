// Pure logic for the one-click join link (#598): parsing and validating a
// boot-time `?room=<CODE>` query parameter, building the shareable URL a
// host's COPY LINK/SHARE actions carry, and computing the address bar a
// guest is left with once that parameter has done its job.
//
// Kept pure and headlessly testable, mirroring `ice_config.ts`'s own split
// for `?ice=relay` (that file's header: "Keep the fetch in the impure
// app-shell layer; pure logic stays testable headlessly"). The impure
// halves -- reading `window.location`, calling `history.replaceState`,
// feature-detecting `navigator.share` -- all live in `browser_main.ts`.
//
// `isRoomCodeShaped` is `@gc/online`'s `room_signaling.ts` export, imported
// rather than re-derived: that module's own header explains why the
// alphabet/length are already duplicated once (against `infra/room_code.ts`,
// which must stay outside this dependency graph) and a second duplication
// here is not owed the same excuse -- `@gc/app` already declares `@gc/online`
// as a dependency (`package.json`), so there is no boundary reason to repeat
// the constants a third time the way `lobby_model.ts`'s own composer has to.

import { isRoomCodeShaped } from "@gc/online";

/**
 * Parses and validates a `?room=<CODE>` query parameter. Returns the
 * uppercased code when `search` carries one shaped like a real room code
 * (`isRoomCodeShaped`), and `undefined` for every other case: the
 * parameter absent, empty, or not shaped like a code at all (junk). A
 * lowercase code is accepted and normalized -- a friend copying a link by
 * hand, or a client that lowercases URLs, must not silently fail here.
 */
export function roomCodeFromSearch(search: string): string | undefined {
  const raw = new URLSearchParams(search).get("room");
  if (raw === null || raw === "") {
    return undefined;
  }
  const candidate = raw.toUpperCase();
  return isRoomCodeShaped(candidate) ? candidate : undefined;
}

/**
 * The full, shareable join URL for a room code, given the page's own
 * origin -- `${origin}/?room=${code}`, the issue's own literal shape.
 * `origin` is a caller-supplied fact (`window.location.origin` in
 * production, an arbitrary string in a spec) so this stays pure: the model
 * that ultimately calls it (`lobby_model.ts`'s `JoinLinkPort`) must never
 * read `window.location` itself.
 */
export function joinUrl(origin: string, code: string): string {
  return `${origin}/?room=${code}`;
}

/**
 * The address bar `history.replaceState` should leave in place once a
 * `?room=<CODE>` parameter has been consumed at boot -- `href` with the
 * `room` parameter stripped, every other parameter/hash left untouched, so
 * a reload or a bookmark of the resulting URL does not re-attempt joining
 * a room that may already be gone (this issue's own acceptance criterion).
 */
export function withoutRoomParam(href: string): string {
  const url = new URL(href);
  url.searchParams.delete("room");
  return `${url.pathname}${url.search}${url.hash}`;
}
