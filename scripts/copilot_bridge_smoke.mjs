// Controlled native local transport smoke; SDK lifecycle events are fixtures.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { randomUUID } from "node:crypto";
const endpoint = `probe-${randomUUID()}`;
const runtime = await fs.mkdtemp(path.join(os.tmpdir(), "treerail-bridge-"));
process.env.XDG_RUNTIME_DIR = runtime;
Object.assign(process.env, {TREERAIL_HOST_ENDPOINT:endpoint,TREERAIL_HOST_SESSION:"local-session",TREERAIL_HOST_TOKEN:"fixture-capability",TREERAIL_COPILOT_SESSION:randomUUID()});
const pipePath = process.platform === "win32" ? `\\\\.\\pipe\\treerail-${endpoint}` : path.join(runtime, `treerail-${endpoint}.sock`);
let count = 0;
const server = net.createServer(socket => {
    let buffer = Buffer.alloc(0);
    socket.on("data", bytes => {
        buffer = Buffer.concat([buffer, bytes]);
        if (buffer.length < 4 || buffer.length < buffer.readUInt32BE(0) + 4) return;
        const message = JSON.parse(buffer.subarray(4).toString("utf8"));
        const observed = message.command.Observe;
        assert.equal(message.version,1); assert.equal(observed.token,"fixture-capability");
        assert.equal(observed.session,"local-session");
        assert.equal(observed.observation.session_id,process.env.TREERAIL_COPILOT_SESSION);
        assert.equal(observed.observation.sequence,++count);
        if(observed.observation.kind==="session.context_changed") assert.equal(observed.observation.cwd,runtime);
        const reply = Buffer.from(JSON.stringify("Done")); const prefix = Buffer.alloc(4); prefix.writeUInt32BE(reply.length);
        // Fragmented replies exercise partial-frame handling.
        socket.write(prefix.subarray(0,2)); socket.write(Buffer.concat([prefix.subarray(2),reply]));
        if (count === 6) { console.log("Private pipe observer framing, identity and sequence passed (controlled SDK events)"); setTimeout(() => process.exit(0),20); }
    });
});
// Begin observation before the endpoint exists to exercise connect-only retry.
setTimeout(() => server.listen(pipePath),150);
const callbacks = new Map();
globalThis.probeJoinSession = async () => ({sessionId:process.env.TREERAIL_COPILOT_SESSION,on:(kind,callback)=>callbacks.set(kind,callback)});
let source = await fs.readFile(new URL("../src/copilot/extension.mjs",import.meta.url),"utf8");
source = source.replace('import { joinSession } from "@github/copilot-sdk/extension";', 'const joinSession = globalThis.probeJoinSession;');
await import(`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`);
callbacks.get("assistant.turn_start")({data:{}});
callbacks.get("elicitation.requested")({data:{requestId:"question"}});
callbacks.get("elicitation.completed")({data:{requestId:"question"}});
callbacks.get("session.context_changed")({data:{cwd:runtime}});
callbacks.get("session.idle")({data:{}});
setTimeout(() => { console.error("Observer smoke timed out"); process.exit(1); },5000);
