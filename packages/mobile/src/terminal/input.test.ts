import { encodeTerminalInput } from "./input";

describe("terminal conversation input", () => {
  it("encodes Unicode terminal input as UTF-8 base64", () => {
    expect(encodeTerminalInput("你好🙂")).toBe(Buffer.from("你好🙂", "utf8").toString("base64"));
  });
});
