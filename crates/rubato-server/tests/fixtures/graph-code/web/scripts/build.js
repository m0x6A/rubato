import { format } from "../src/format.js";

function build() {
  return format("x");
}

class Runner {
  run() {
    return build();
  }
}

export { Runner };
