// Controlled SDK transport: this proves our admission/integrity/replay rules,
// not native Copilot compatibility. No provider/model work is started.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createHash, randomUUID } from "node:crypto";

const directory = fs.mkdtempSync(path.join(os.tmpdir(), "treerail-discussion-"));
const document = path.join(directory, "approved.md");
const content = `# Approved task\n${"Full comment with Unicode ✓\n".repeat(5000)}LAST-COMMENT`;
fs.writeFileSync(document, content);
Object.assign(process.env, {
    TREERAIL_HOST_ENDPOINT:"controlled-document", TREERAIL_HOST_SESSION:"local", TREERAIL_HOST_TOKEN:"token",
    TREERAIL_COPILOT_SESSION:randomUUID(), XDG_RUNTIME_DIR:directory,
    TREERAIL_DISCUSSION_PATH:document, TREERAIL_DISCUSSION_SHA256:createHash("sha256").update(content).digest("hex"),
    TREERAIL_DISCUSSION_BYTES:String(Buffer.byteLength(content)),
});
let sent = [];
globalThis.probeJoinSession = async () => ({
    sessionId:process.env.TREERAIL_COPILOT_SESSION, on:()=>{},
    send:async options => { sent.push(options.prompt); return "message-identity"; },
});
let source = fs.readFileSync(new URL("../src/copilot/extension.mjs",import.meta.url),"utf8")
    .replace('import { joinSession } from "@github/copilot-sdk/extension";', 'const joinSession = globalThis.probeJoinSession;')
    // Observer transport is independently covered by copilot_bridge_smoke.mjs.
    .replace('send("bridge.connected");', '')
    .replace('setInterval(() => send("bridge.heartbeat"), 1000);', '');
const run = () => import(`data:text/javascript;base64,${Buffer.from(source+`\n// ${randomUUID()}`).toString("base64")}`);
await run();
assert.equal(sent.length,1);
assert.equal(sent[0],content);
assert(Buffer.byteLength(sent[0])>48*1024);
const marker = `${document}.${process.env.TREERAIL_COPILOT_SESSION}.delivery`;
assert.equal(fs.readFileSync(marker,"utf8"),"admitted message-identity\n");
await assert.rejects(run(),/already attempted/);
assert.equal(sent.length,1,"Extension reload must not replay initial context");
fs.writeFileSync(document,"Changed text");
await assert.rejects(run(),/changed/);
assert.equal(sent.length,1);
fs.writeFileSync(document,content);
process.env.TREERAIL_COPILOT_SESSION=randomUUID();
globalThis.probeJoinSession = async () => ({sessionId:process.env.TREERAIL_COPILOT_SESSION,on:()=>{},send:async () => {throw new Error("Lost admission response");}});
await assert.rejects(run(),/Lost admission response/);
const uncertain=`${document}.${process.env.TREERAIL_COPILOT_SESSION}.delivery`;
assert.equal(fs.readFileSync(uncertain,"utf8"),"confirmation unknown\n");
await assert.rejects(run(),/already attempted/);
fs.rmSync(directory,{recursive:true});
console.log("Complete large document, changed content, reload and uncertain-admission checks passed (controlled SDK)");
