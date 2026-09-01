import { describe, expect, it } from "vitest";
import { formatPairingCode, remembersHttpApproval, REQUIRED_SCOPES } from "./connection";

describe("connection onboarding", () => {
  it("formats numeric and alphanumeric pairing codes", () => {
    expect(formatPairingCode("jm38mhsa")).toBe("JM38-MHSA");
    expect(formatPairingCode("1234-5678")).toBe("1234-5678");
    expect(formatPairingCode("jm38!mhsa-extra")).toBe("JM38-MHSA");
  });

  it("remembers HTTP approval only for the exact origin", () => {
    expect(remembersHttpApproval("http://romm.local/library", "http://romm.local", true)).toBe(true);
    expect(remembersHttpApproval("http://other.local", "http://romm.local", true)).toBe(false);
    expect(remembersHttpApproval("not a url", "http://romm.local", true)).toBe(false);
  });

  it("includes identity and every v1 feature scope", () => {
    expect(REQUIRED_SCOPES).toEqual([
      "me.read",
      "roms.read",
      "platforms.read",
      "collections.read",
      "roms.user.read",
      "roms.user.write",
      "assets.read",
      "assets.write",
      "devices.read",
      "devices.write",
    ]);
  });
});
