// Opt-in proof observer. Installed only into a disposable probe worktree.
// Never approves permission, answers questions, or submits agent prompts.
import { joinSession } from "@github/copilot-sdk/extension";
import net from "node:net";
const endpoint = process.env.TREERAIL_PROBE_ADDRESS;
const token = process.env.TREERAIL_PROBE_TOKEN;
const expected = process.env.TREERAIL_PROBE_SESSION;
if (!endpoint || !token || !expected) throw new Error("Missing probe connection context");
const [host, port] = endpoint.split(":");
let sequence = 0;
let queue = Promise.resolve();
let pending = 0;
function send(kind, requestId) {
    if (++pending > 64) process.exit(1);
    const observation = {session_id: expected, sequence: ++sequence, kind, request_id: requestId ?? null};
    queue = queue.then(() => new Promise((resolve, reject) => {
        const socket = net.createConnection({host, port: Number(port)});
        socket.setTimeout(3000, () => socket.destroy(new Error("Probe connection timed out")));
        socket.once("error", reject);
        socket.once("connect", () => socket.write(JSON.stringify({token, command: "observe", observation}) + "\n"));
        socket.once("data", () => { pending--; socket.end(); resolve(); });
    })).catch(() => process.exit(1));
}
const session = await joinSession({});
if (session.sessionId !== expected) throw new Error("Provider conversation identity differs from requested UUID");
send("bridge.connected");
for (const kind of ["assistant.turn_start", "session.idle", "session.error", "permission.requested", "permission.completed", "user_input.requested", "user_input.completed", "elicitation.requested", "elicitation.completed"]) {
    session.on(kind, event => { if (!event.agentId) send(kind, event.data?.requestId); });
}
// Heartbeats prove event-bridge liveness, never model activity.
setInterval(() => send("bridge.heartbeat"), 1000);
