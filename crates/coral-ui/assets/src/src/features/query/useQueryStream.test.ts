import { describe, expect, it } from "vitest";
import { parseFrames } from "./useQueryStream";

describe("parseFrames (SSE wire format)", () => {
  it("parses a single complete frame", () => {
    const { frames, rest } = parseFrames(
      "event: token\ndata: {\"text\":\"hello\"}\n\n",
    );
    expect(frames).toEqual([{ event: "token", data: '{"text":"hello"}' }]);
    expect(rest).toBe("");
  });

  it("buffers a half-frame until the terminating \\n\\n arrives", () => {
    const first = parseFrames("event: token\ndata: {\"text\":\"hel");
    expect(first.frames).toEqual([]);
    expect(first.rest).toBe('event: token\ndata: {"text":"hel');
    const second = parseFrames(first.rest + 'lo"}\n\n');
    expect(second.frames).toEqual([{ event: "token", data: '{"text":"hello"}' }]);
    expect(second.rest).toBe("");
  });

  it("emits multiple frames in one chunk", () => {
    const { frames } = parseFrames(
      "event: token\ndata: {\"text\":\"a\"}\n\nevent: token\ndata: {\"text\":\"b\"}\n\nevent: done\ndata: {}\n\n",
    );
    expect(frames).toHaveLength(3);
    expect(frames[0]).toEqual({ event: "token", data: '{"text":"a"}' });
    expect(frames[1]).toEqual({ event: "token", data: '{"text":"b"}' });
    expect(frames[2]).toEqual({ event: "done", data: "{}" });
  });

  it("defaults event to \"message\" when omitted", () => {
    const { frames } = parseFrames("data: {\"text\":\"hi\"}\n\n");
    expect(frames).toEqual([{ event: "message", data: '{"text":"hi"}' }]);
  });

  it("joins multi-line data with \\n per SSE spec", () => {
    const { frames } = parseFrames(
      "event: token\ndata: line one\ndata: line two\n\n",
    );
    expect(frames).toEqual([{ event: "token", data: "line one\nline two" }]);
  });

  it("ignores blocks with no data: field", () => {
    const { frames } = parseFrames("event: ping\n\n");
    expect(frames).toEqual([]);
  });

  it("trims leading whitespace after the data: prefix", () => {
    const { frames } = parseFrames(
      "event: source\ndata:   {\"slug\":\"x\"}\n\n",
    );
    expect(frames[0].data).toBe('{"slug":"x"}');
  });

  it("returns the partial trailing block intact for the next chunk", () => {
    const { rest } = parseFrames(
      "event: token\ndata: {\"text\":\"first\"}\n\nevent: token\ndata: partial",
    );
    expect(rest).toBe("event: token\ndata: partial");
  });

  it("survives an empty buffer", () => {
    expect(parseFrames("")).toEqual({ frames: [], rest: "" });
  });
});
