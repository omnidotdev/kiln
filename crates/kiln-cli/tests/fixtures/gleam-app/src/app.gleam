import gleam/erlang/process
import gleam/io

// Minimal app: the erlang-shipment's entrypoint.sh must boot this (the runtime
// image has no `gleam` binary). It prints and then stays alive.
pub fn main() {
  io.println("ok")
  process.sleep_forever()
}
