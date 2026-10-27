const net = require("node:net");
const tls = require("node:tls");
const dgram = require("node:dgram");
const dns = require("node:dns");
const { syncBuiltinESMExports } = require("node:module");

let violations = 0;
const denied = () => {
  violations += 1;
  const error = new Error("Offline verification blocked an unmocked transport");
  error.code = "REFACT_OFFLINE_NETWORK";
  throw error;
};

net.Socket.prototype.connect = denied;
net.connect = denied;
net.createConnection = denied;
tls.connect = denied;
dgram.Socket.prototype.connect = denied;
dgram.Socket.prototype.send = denied;

for (const name of Object.keys(dns)) {
  if (name === "lookup" || name === "reverse" || name.startsWith("resolve")) {
    if (typeof dns[name] === "function") dns[name] = denied;
    if (typeof dns.promises[name] === "function") dns.promises[name] = denied;
  }
}

syncBuiltinESMExports();
Object.defineProperty(globalThis, "__REFACT_OFFLINE_NETWORK__", {
  value: true,
  writable: false,
  configurable: false,
});
Object.defineProperty(globalThis, "__REFACT_OFFLINE_VIOLATIONS__", {
  get: () => violations,
  configurable: false,
});
process.once("exit", () => {
  process.stderr.write(
    JSON.stringify({
      offline_transport_audit: true,
      pid: process.pid,
      violations,
    }) + "\n",
  );
});
