// Opt-in native compatibility proof. No cloud API, credentials, inference or AI credits.
// Requires a real Windows terminal and installed Copilot 1.0.94; never run by CI.
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),http=require('node:http'),net=require('node:net'),cp=require('node:child_process');
if(process.platform!=='win32'||!process.stdin.isTTY||!process.stdout.isTTY)throw new Error('Run this opt-in proof in a real Windows terminal.'); const version=cp.execFileSync('copilot',['--no-auto-update','--version'],{encoding:'utf8',timeout:15000});if(!version.includes('GitHub Copilot CLI 1.0.94.'))throw new Error('This proof targets Copilot 1.0.94 only.'); const root=path.resolve('target/native-discussion-proof',crypto.randomUUID());fs.mkdirSync(root,{recursive:true});
const home=path.join(root,'offline-home-'+crypto.randomUUID()); const ext=path.join(home,'extensions','treerail-session-host');fs.mkdirSync(ext,{recursive:true});
fs.copyFileSync('src/copilot/extension.mjs',path.join(ext,'extension.mjs'));
const id=crypto.randomUUID(),endpoint='context-proof-'+crypto.randomUUID();
const text='# Frozen discussion delivery proof\n\nDo not use tools. No response is required.\n'+('A complete preserved discussion line. '.repeat(2300))+'\nFINAL-CONTEXT-MARKER-'+id;
const doc=path.join(root,id+'.md');fs.writeFileSync(doc,text);
const digest=crypto.createHash('sha256').update(text).digest('hex');
const result={mode:'offline-loopback-only',session:id,version:'1.0.94',bytes:Buffer.byteLength(text),sha256:digest,observations:[],admitted:false,exactUserMessage:false};
const server=net.createServer(socket=>{let buffer=Buffer.alloc(0);socket.on('data',chunk=>{buffer=Buffer.concat([buffer,chunk]);if(buffer.length<4||buffer.length<4+buffer.readUInt32BE(0))return;try{const msg=JSON.parse(buffer.subarray(4,4+buffer.readUInt32BE(0)));result.observations.push(msg.command.Observe.observation.kind);const reply=Buffer.from('"Done"'),prefix=Buffer.alloc(4);prefix.writeUInt32BE(reply.length);socket.end(Buffer.concat([prefix,reply]));}catch{socket.destroy();}});});
const modelServer=http.createServer((req,res)=>{let body='';req.on('data',bytes=>body+=bytes);req.on('end',()=>{let data;try{data=JSON.parse(body)}catch{}result.localProviderRequest=true;const strings=value=>typeof value==='string'?[value]:value&&typeof value==='object'?Object.values(value).flatMap(strings):[];result.fullDocumentAtLocalProvider=!!result.fullDocumentAtLocalProvider||strings(data).some(value=>value.includes(text));res.writeHead(200,{'content-type':'application/json'});res.end(JSON.stringify({id:'local-free',object:'chat.completion',created:1,model:'local-free-proof',choices:[{index:0,message:{role:'assistant',content:'Local transport proof complete.'},finish_reason:'stop'}],usage:{prompt_tokens:0,completion_tokens:0,total_tokens:0}}));});}); modelServer.listen(0,'127.0.0.1',()=>server.listen('\\\\.\\pipe\\treerail-'+endpoint,()=>{
 const env={...process.env,COPILOT_HOME:home,COPILOT_AUTO_UPDATE:'false',TREERAIL_HOST_ENDPOINT:endpoint,TREERAIL_HOST_SESSION:'proof',TREERAIL_HOST_TOKEN:'proof',TREERAIL_COPILOT_SESSION:id,TREERAIL_DISCUSSION_PATH:doc,TREERAIL_DISCUSSION_SHA256:digest,TREERAIL_DISCUSSION_BYTES:String(Buffer.byteLength(text)),USE_TGREP:'false'};

 for(const key of Object.keys(env))if(key.startsWith('COPILOT_PROVIDER_')||['GH_TOKEN','GITHUB_TOKEN','COPILOT_GITHUB_TOKEN'].includes(key))delete env[key]; Object.assign(env,{COPILOT_OFFLINE:'true',NO_PROXY:'127.0.0.1,localhost,::1',HTTP_PROXY:'',HTTPS_PROXY:'',ALL_PROXY:'',http_proxy:'',https_proxy:'',all_proxy:'',COPILOT_PROVIDER_TYPE:'openai',COPILOT_PROVIDER_BASE_URL:'http://127.0.0.1:'+modelServer.address().port+'/v1',COPILOT_PROVIDER_MODEL_ID:'gpt-4.1',COPILOT_PROVIDER_WIRE_MODEL:'local-free-proof'}); fs.writeFileSync(path.join(home,'config.json'),JSON.stringify({trusted_folders:[root],auto_update:false})); const child=cp.spawn('copilot',['--no-auto-update','--experimental','--session-id',id,'--model','gpt-4.1','--no-remote','--no-remote-export'],{cwd:root,env,windowsHide:true,stdio:'inherit'});
 const log=fs.createWriteStream(path.join(root,'cli-output.log'));if(child.stdout)child.stdout.pipe(log);if(child.stderr)child.stderr.pipe(log);
 const timer=setInterval(()=>{
  const marker=doc+'.'+id+'.delivery';if(fs.existsSync(marker))result.delivery=fs.readFileSync(marker,'utf8').trim();
  const eventPath=path.join(home,'session-state',id,'events.jsonl');
  if(fs.existsSync(eventPath)){
   const events=fs.readFileSync(eventPath,'utf8').split('\n').filter(Boolean).flatMap(line=>{try{return[JSON.parse(line)]}catch{return[]}});
   const message=events.find(event=>event.type==='user.message'&&(event.data?.content===text||event.data?.transformedContent===text));
   result.userMessageCount=events.filter(event=>event.type==='user.message').length;result.exactUserMessage=!!message&&result.userMessageCount===1;
  }
  result.admitted=!!result.delivery?.startsWith('admitted ');
  if(result.admitted&&result.exactUserMessage&&result.fullDocumentAtLocalProvider)finish();
 },500);
 const deadline=setTimeout(finish,35000);let finishing=false;
 function finish(){if(finishing)return;finishing=true;clearInterval(timer);clearTimeout(deadline);fs.writeFileSync(path.join(root,'result.json'),JSON.stringify(result,null,2));cp.spawnSync('taskkill',['/PID',String(child.pid),'/T','/F'],{windowsHide:true,stdio:'ignore'});server.close();modelServer.close();console.log(JSON.stringify(result));setTimeout(()=>process.exit(result.admitted&&result.exactUserMessage&&result.fullDocumentAtLocalProvider?0:1),200);}
 child.on('error',error=>{result.error=error.message;finish();});
}));