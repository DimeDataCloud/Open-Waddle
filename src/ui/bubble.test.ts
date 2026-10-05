import { describe, expect, it } from "vitest";
import { plain } from "./bubble";

describe("plain", () => {
  it("drops the Markdown models sometimes answer in", () => {
    expect(plain("The subject is **Offsite agenda**.")).toBe("The subject is Offsite agenda.");
    expect(plain("Created `hello.txt` containing `hi`.")).toBe("Created hello.txt containing hi.");
    expect(plain("## Cost\nSee [bbc.co.uk](https://bbc.co.uk/x).")).toBe("Cost\nSee bbc.co.uk.");
    expect(plain("2 * 3 = 6, snake_case_name")).toBe("2 * 3 = 6, snake_case_name");
    expect(plain("Tips:\n*   **Short:** yes\n- Clear\n  + nested")).toBe("Tips:\n• Short: yes\n• Clear\n  • nested");
  });
});
