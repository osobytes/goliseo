// The one place `@gc/wasm` converts between a `Uint8Array` (how bytes cross
// the wasm boundary — `wasm-bindgen`'s native, lossless `Vec<u8>`/`&[u8]`
// marshalling, see `crates/gc-wasm/src/input_protocol_bridge.rs`'s and
// `crates/gc-wasm/src/match_driver_fixture_bridge.rs`'s module docs) and a
// "binary string" (one JS UTF-16 code unit per byte, code points 0-255
// only) — the convention `@gc/transport`'s `TransportMessage.payload`
// already uses (`packages/transport/src/contract.ts`'s own doc comment) and
// `packages/online/src/net_diagnostics_fixture.ts`'s `InputProtocolPort.encode`
// declares its return type as (`Result<string, string>`).
//
// Never `TextEncoder`/`TextDecoder` (UTF-8): a wire payload is not
// necessarily valid UTF-8 text (a Lua string, and the protocol byte streams
// this package binds, are raw byte arrays — see the Rust modules' own
// docs), and a UTF-8 round trip silently corrupts any byte outside that
// encoding's rules. `String.fromCharCode`/`charCodeAt` operate on raw
// UTF-16 code units and never attempt to interpret the bytes as text, so
// they are the only correct conversion here.

/** Converts raw bytes to a "binary string": one UTF-16 code unit per byte. */
export function byteStringFromBytes(bytes: Uint8Array): string {
  let out = "";
  // Chunked to avoid `String.fromCharCode(...bytes)` blowing the call stack
  // on a large array — no wire this package binds is anywhere near this
  // size, but the helper is otherwise general-purpose.
  const chunkSize = 4096;
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    const chunk = bytes.subarray(offset, Math.min(offset + chunkSize, bytes.length));
    out += String.fromCharCode(...chunk);
  }
  return out;
}

/**
 * The reverse of {@link byteStringFromBytes}. Every code unit must be in
 * `0..255` — anything else means `value` was never a binary string (real
 * Unicode text got here by mistake), and that is a caller bug worth
 * throwing on rather than silently truncating to a byte.
 *
 * @throws If `value` contains a code unit outside `0..255`.
 */
export function bytesFromByteString(value: string): Uint8Array {
  const bytes = new Uint8Array(value.length);
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code > 0xff) {
      throw new Error(
        `binary string has a code unit outside 0..255 at index ${index} (got ${code}) -- ` +
          "this is not a binary string; UTF-8 text must be encoded to bytes before this call",
      );
    }
    bytes[index] = code;
  }
  return bytes;
}
