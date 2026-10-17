// Native Copilot observer. The CLI resolves its own bundled SDK import.
// No prompts, tools, permission callbacks or automated responses are registered.
import { joinSession } from "@github/copilot-sdk/extension";
import net from "node:net";
import path from "node:path";
const endpoint = process.env.TREERAIL_HOST_ENDPOINT;
const localSession = process.env.TREERAIL_HOST_SESSION;
const token = process.env.TREERAIL_HOST_TOKEN;
const expected = process.env.TREERAIL_COPILOT_SESSION;
if (![endpoint, localSession, token, expected].some(Boolean)) process.exit(0);
if (!endpoint || !localSession || !token || !expected) throw new Error("Missing Tree Rail host context");
if (!/^[a-zA-Z0-9-]{1,100}$/.test(endpoint)) throw new Error("Invalid host endpoint");
const pipePath = process.platform === "win32"
    ? `\\\\.\\pipe\\treerail-${endpoint}`
    : path.join(process.env.XDG_RUNTIME_DIR ?? "", `treerail-${endpoint}.sock`);
if (process.platform !== "win32" && !process.env.XDG_RUNTIME_DIR) throw new Error("Missing private runtime directory");
let sequence = 0;
let pending = 0;
let queue = Promise.resolve();
async function connectHost() {
    const deadline = Date.now() + 3000;
    for (;;) {
        try {
            return await new Promise((resolve, reject) => {
                const socket = net.createConnection(pipePath);
                socket.setTimeout(3000, () => { const error=new Error("Tree Rail host connect timed out"); error.code="ETIMEDOUT"; socket.destroy(error); });
                const failed = error => { socket.destroy(); reject(error); };
                socket.once("error", failed);
                socket.once("connect", () => { socket.removeListener("error", failed); socket.setTimeout(0); resolve(socket); });
            });
        } catch (error) {
            if (!["ENOENT", "EBUSY", "ECONNREFUSED"].includes(error.code) || Date.now() >= deadline) throw error;
            // Connection-only retry. Never replay an observation after writing bytes.
            await new Promise(resolve => setTimeout(resolve, 10));
        }
    }
}
function send(kind, requestId, cwd) {
    if (++pending > 64) process.exit(1);
    const observation = {session_id: expected, sequence: ++sequence, kind, request_id: requestId ?? null, cwd: cwd ?? null};
    queue = queue.then(async () => {
        const socket = await connectHost();
        return new Promise((resolve, reject) => {
        socket.setTimeout(3000, () => socket.destroy(new Error("Tree Rail host timed out")));
        socket.once("error", reject);
        let received = Buffer.alloc(0);
        {
            const json = Buffer.from(JSON.stringify({version: 1, command: {Observe: {session: localSession, token, observation}}}));
            const prefix = Buffer.alloc(4); prefix.writeUInt32BE(json.length);
            socket.write(Buffer.concat([prefix, json]));
        }
        socket.on("data", bytes => {
            received = Buffer.concat([received, bytes]);
            if (received.length > 65540) { socket.destroy(); reject(new Error("Oversized host response")); return; }
            if (received.length < 4) return;
            const length = received.readUInt32BE(0);
            if (length > 65536) { socket.destroy(); reject(new Error("Oversized host response")); return; }
            if (received.length < length + 4) return;
            try {
                const response = JSON.parse(received.subarray(4, length + 4).toString("utf8"));
                if (response !== "Done") throw new Error("Host rejected provider observation");
                pending--; socket.end(); resolve();
            } catch (error) { socket.destroy(); reject(error); }
        });
        socket.once("end", () => { if (received.length < 4 || received.length < received.readUInt32BE(0) + 4) reject(new Error("Incomplete host response")); });
        });
    }).catch(() => process.exit(1));
}
const session = await joinSession({});
if (session.sessionId !== expected) throw new Error("Copilot conversation identity differs from recorded UUID");
send("bridge.connected");
for (const kind of ["assistant.turn_start", "session.idle", "session.error", "permission.requested", "permission.completed", "user_input.requested", "user_input.completed", "elicitation.requested", "elicitation.completed", "session.context_changed"]) {
    session.on(kind, event => { if (!event.agentId) send(kind, event.data?.requestId, event.data?.cwd); });
}
setInterval(() => send("bridge.heartbeat"), 1000);
