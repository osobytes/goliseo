// Ported from spec/game/transport_spec.lua.

import { describe, expect, it } from "vitest";
import * as contract from "./contract.ts";
import * as transport from "./index.ts";
import type { FakeTransport } from "./fake.ts";
import type { TransportFailure, TransportMessage, TransportResult } from "./contract.ts";
import type { EvalFn } from "./browser.ts";

function unwrap<T>(result: TransportResult<T>): T {
  if (!result.ok) {
    throw new Error(`${result.error.code}: ${result.error.message}`);
  }
  return result.value;
}

function expectErr<T>(result: TransportResult<T>): TransportFailure {
  expect(result.ok).toBe(false);
  if (result.ok) {
    throw new Error("expected a transport failure");
  }
  return result.error;
}

function message(seq: number, tick?: number, payload = ""): TransportMessage {
  return unwrap(
    contract.newMessage({
      type: tick !== undefined ? "input" : "event",
      seq,
      ...(tick !== undefined ? { tick } : {}),
      payload,
    })
  );
}

function fakeBrowserHost(fake: FakeTransport): EvalFn {
  return (command: string) => {
    const match = /^window\.GoliseoTransportBridge\.([A-Za-z0-9_]+)\((.*)\)$/.exec(command);
    if (!match) {
      throw new Error(`unexpected browser command: ${command}`);
    }
    const name = match[1] as string;
    const argument = match[2] as string;
    if (name === "initialize") {
      unwrap(fake.initialize());
      return ["state|connected", null] as const;
    } else if (name === "shutdown") {
      unwrap(fake.shutdown());
      return ["state|closed", null] as const;
    } else if (name === "enqueue") {
      const wireMatch = /^'([\s\S]*)'$/.exec(argument);
      if (!wireMatch) {
        throw new Error(`unexpected enqueue argument: ${argument}`);
      }
      const decoded = unwrap(contract.decode(wireMatch[1] as string));
      const result = fake.enqueue(decoded);
      if (result.ok) {
        return ["ok", null] as const;
      }
      return [`error|${result.error.code}|${result.error.message}`, null] as const;
    } else if (name === "poll") {
      const result = fake.poll();
      const value = result.ok ? result.value : null;
      return [value !== null ? unwrap(contract.encode(value)) : "", null] as const;
    } else if (name === "poll_event") {
      const event = fake.pollEvent();
      if (!event) {
        return ["", null] as const;
      }
      if (event.kind === "state") {
        return [`state|${event.state}`, null] as const;
      }
      return [`error|${event.code}`, null] as const;
    } else if (name === "disconnect") {
      fake.disconnect();
      return ["state|disconnected", null] as const;
    } else if (name === "diagnostics") {
      const d = fake.diagnostics();
      return [
        [
          d.state,
          d.queue_limit,
          d.outbound_depth,
          d.inbound_depth,
          d.event_depth,
          d.dropped_outbound,
          d.dropped_inbound,
          d.malformed,
          d.unsupported_version,
          d.overflow,
          d.sent,
          d.received,
          d.last_error ?? "",
        ].join("|"),
        null,
      ] as const;
    }
    throw new Error(`unexpected browser method: ${name}`);
  };
}

describe("transport envelope", () => {
  it("validates and round-trips a tick-numbered input", () => {
    const original = message(7, 42, "left|right\n%\xff");
    const wire = unwrap(contract.encode(original));
    const decoded = unwrap(contract.decode(wire));
    expect(decoded.version).toBe(1);
    expect(decoded.type).toBe("input");
    expect(decoded.seq).toBe(7);
    expect(decoded.tick).toBe(42);
    expect(decoded.payload).toBe(original.payload);
  });

  it("rejects malformed, unsupported, and oversized messages", () => {
    const malformed = contract.decode("1|input|not-a-seq|2|payload");
    expect(malformed.ok).toBe(false);
    expect(expectErr(malformed).code).toBe("malformed");

    const unsupported = contract.newMessage({
      version: 2,
      type: "input",
      seq: 1,
      tick: 1,
      payload: "payload",
    });
    expect(expectErr(unsupported).code).toBe("unsupported_version");

    const oversized = contract.newMessage({
      type: "event",
      seq: 1,
      payload: "x".repeat(contract.MAX_PAYLOAD_BYTES + 1),
    });
    expect(expectErr(oversized).code).toBe("payload_too_large");
  });

  it("decodes a control envelope that carries no tick", () => {
    const original = unwrap(contract.newMessage({ type: "event", seq: 4, payload: "lifecycle" }));
    const decoded = unwrap(contract.decode(unwrap(contract.encode(original))));
    expect(decoded.type).toBe("event");
    expect(decoded.seq).toBe(4);
    expect(decoded.tick).toBeUndefined();
    expect(decoded.payload).toBe("lifecycle");
  });

  it("requires ticks for input messages but permits control messages without one", () => {
    const input = contract.newMessage({ type: "input", seq: 1, payload: "move" });
    expect(expectErr(input).code).toBe("malformed");

    const event = unwrap(contract.newMessage({ type: "event", seq: 1, payload: "connected" }));
    expect(event.tick).toBeUndefined();
  });
});

describe("fake loopback transport", () => {
  it("initializes, loops back, and drains inbound messages in order", () => {
    const fake = transport.fake();
    expect(fake.state()).toBe("new");
    expect(unwrap(fake.initialize())).toBe(true);
    expect(fake.state()).toBe("connected");
    expect(unwrap(fake.enqueue(message(1, 10, "a")))).toBe(true);
    expect(unwrap(fake.enqueue(message(2, 11, "b")))).toBe(true);
    expect(unwrap(fake.enqueue(message(3, 12, "c")))).toBe(true);

    expect(unwrap(fake.poll())?.seq).toBe(1);
    expect(unwrap(fake.poll())?.seq).toBe(2);
    expect(unwrap(fake.poll())?.seq).toBe(3);
    expect(unwrap(fake.poll())).toBeNull();
    expect(fake.diagnostics().sent).toBe(3);
    expect(fake.diagnostics().received).toBe(3);
  });

  it("reports bounded queue depth and overflow without blocking", () => {
    const fake = transport.fake({ queue_limit: 2 });
    unwrap(fake.initialize());
    unwrap(fake.inject(message(1, 1, "inbound-a")));
    unwrap(fake.inject(message(2, 2, "inbound-b")));
    unwrap(fake.enqueue(message(3, 3, "outbound-a")));
    unwrap(fake.enqueue(message(4, 4, "outbound-b")));
    const overflowed = fake.enqueue(message(5, 5, "outbound-c"));
    expect(expectErr(overflowed).code).toBe("overflow");

    const diagnostics = fake.diagnostics();
    expect(diagnostics.queue_limit).toBe(2);
    expect(diagnostics.inbound_depth).toBe(2);
    expect(diagnostics.outbound_depth).toBe(2);
    expect(diagnostics.overflow).toBe(1);
    expect(diagnostics.dropped_outbound).toBe(1);

    expect(unwrap(fake.poll())?.seq).toBe(1);
    expect(unwrap(fake.poll())?.seq).toBe(2);
    expect(unwrap(fake.poll())?.seq).toBe(3);
    expect(unwrap(fake.poll())?.seq).toBe(4);
  });

  it("exposes malformed, unsupported-version, disconnect, and shutdown events", () => {
    const fake = transport.fake();
    unwrap(fake.initialize());
    const malformedMessage = {
      version: 1,
      type: "bogus",
      seq: 1,
      payload: "bad",
    } as unknown as TransportMessage;
    const malformed = fake.inject(malformedMessage);
    expect(expectErr(malformed).code).toBe("malformed");

    const unsupported = fake.inject({
      version: 99,
      type: "event",
      seq: 2,
      payload: "old",
    } as unknown as TransportMessage);
    expect(expectErr(unsupported).code).toBe("unsupported_version");
    expect(fake.pollEvent()?.state).toBe("connected");
    expect(fake.pollEvent()?.code).toBe("malformed");
    expect(fake.pollEvent()?.code).toBe("unsupported_version");

    unwrap(fake.disconnect("peer closed"));
    expect(fake.state()).toBe("disconnected");
    expect(fake.pollEvent()?.state).toBe("disconnected");
    expect(fake.pollEvent()?.code).toBe("disconnected");

    unwrap(fake.shutdown());
    expect(fake.state()).toBe("closed");
    expect(fake.pollEvent()?.state).toBe("closed");
  });

  it("bounds the observable event queue", () => {
    const fake = transport.fake({ queue_limit: 2 });
    unwrap(fake.initialize());

    for (let seq = 1; seq <= 3; seq += 1) {
      const malformedMessage = {
        version: 1,
        type: "invalid",
        seq,
        payload: "bad",
      } as unknown as TransportMessage;
      const result = fake.inject(malformedMessage);
      expect(expectErr(result).code).toBe("malformed");
    }

    const diagnostics = fake.diagnostics();
    expect(diagnostics.event_depth).toBe(2);
    expect(diagnostics.overflow).toBeGreaterThanOrEqual(1);
    expect(fake.pollEvent()?.code).toBe("malformed");
    expect(fake.pollEvent()?.code).toBe("malformed");
    expect(fake.pollEvent()).toBeNull();
  });

  it("retains the disconnect state/error pair at the minimum queue limit", () => {
    const fake = transport.fake({ queue_limit: 1 });
    unwrap(fake.initialize());

    unwrap(fake.disconnect("peer closed"));
    expect(fake.diagnostics().event_depth).toBe(2);
    expect(fake.pollEvent()?.state).toBe("disconnected");
    expect(fake.pollEvent()?.code).toBe("disconnected");
    expect(fake.pollEvent()).toBeNull();
  });

  it("drops queued messages on disconnect before reconnecting", () => {
    const fake = transport.fake({ queue_limit: 2 });
    unwrap(fake.initialize());
    fake.pollEvent();
    unwrap(fake.inject(message(1, 1, "inbound-a")));
    unwrap(fake.inject(message(2, 2, "inbound-b")));
    unwrap(fake.enqueue(message(3, 3, "outbound")));

    unwrap(fake.disconnect("peer closed"));
    const disconnected = fake.diagnostics();
    expect(disconnected.dropped_inbound).toBe(2);
    expect(disconnected.dropped_outbound).toBe(1);
    expect(disconnected.inbound_depth).toBe(0);
    expect(disconnected.outbound_depth).toBe(0);
    expect(fake.pollEvent()?.state).toBe("disconnected");
    expect(fake.pollEvent()?.code).toBe("disconnected");

    unwrap(fake.initialize());
    expect(fake.pollEvent()?.state).toBe("connected");
    expect(unwrap(fake.poll())).toBeNull();
  });
});

describe("browser transport contract", () => {
  it("uses the same enqueue/poll behavior through the host seam", () => {
    const fake = transport.fake();
    const browser = transport.browser({ eval: fakeBrowserHost(fake) });
    expect(unwrap(browser.initialize())).toBe(true);
    unwrap(browser.enqueue(message(8, 80, "first")));
    unwrap(browser.enqueue(message(9, 81, "second")));
    expect(unwrap(browser.poll())?.seq).toBe(8);
    expect(unwrap(browser.poll())?.seq).toBe(9);
    expect(browser.diagnostics().sent).toBe(2);
    expect(browser.diagnostics().received).toBe(2);
  });

  it("keeps connection transitions and queue diagnostics observable", () => {
    const fake = transport.fake({ queue_limit: 1 });
    const browser = transport.browser({
      queue_limit: 1,
      eval: fakeBrowserHost(fake),
    });
    unwrap(browser.initialize());
    expect(browser.pollEvent()?.state).toBe("connected");
    unwrap(browser.disconnect("test"));
    expect(browser.state()).toBe("disconnected");
    expect(browser.pollEvent()?.state).toBe("disconnected");
    expect(browser.pollEvent()?.code).toBe("disconnected");
    unwrap(browser.shutdown());
    expect(browser.state()).toBe("closed");
    expect(browser.pollEvent()?.state).toBe("closed");
  });
});
