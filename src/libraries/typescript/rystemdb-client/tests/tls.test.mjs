import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { mkdtemp, rm, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { RystemLocalClient } from "../src/client.ts";
import { RdbReference } from "../src/rdb.ts";

const fixtures = fileURLToPath(new URL("../../../../rystemdb-cli/tests/fixtures/tls/", import.meta.url));
const tokenR = Uint8Array.from({length:32},()=>0x11);
const tokenW = Uint8Array.from({length:32},()=>0x22);
function document() {
  return new Map([
    ["bytes",Uint8Array.from([0,255,128])],
    ["reference",new RdbReference("users","u42")],
    ["array",[-9223372036854775808n,null,true]],
  ]);
}
async function startNative(binary, data, auth, cert, key) {
  const child = spawn(binary, ["rpc-serve","--data",data,"--listen","127.0.0.1:0",
    "--auth-file",auth,"--tls-cert",cert,"--tls-key",key],{
    stdio:["ignore","pipe","pipe"],
  });
  try {
    const rl=createInterface({input:child.stdout});
    const [banner]=await Promise.race([
      once(rl,"line"),
      once(child,"exit").then(([code])=>{throw new Error("TLS server exited "+code);}),
      new Promise((_,reject)=>{
        const timer=setTimeout(()=>reject(new Error("TLS startup timeout")),20000);
        timer.unref();
      }),
    ]);
    rl.close();
    assert.match(banner,/LOOPBACK RPC listening on 127\.0\.0\.1:\d+$/);
    return {child,port:Number(banner.match(/:(\d+)$/)[1])};
  } catch(err) {
    child.kill();
    throw err;
  }
}
async function stopNative(server) {
  if(!server.child.killed) server.child.kill();
  await Promise.race([
    once(server.child,"exit").catch(()=>{}),
    new Promise(resolve=>{
      const t=setTimeout(resolve,2000);t.unref();
    }),
  ]);
}

test("TDD RED #52: real Rust server TLS1.3 verified Node binary CRUD and restart", {
  skip:!process.env.RYSTEMDB_BINARY,timeout:60000,
},async()=>{
  const root=await mkdtemp(join(tmpdir(),"rystem-node-tls-"));
  let server;
  try {
    const acl=join(root,"acl.txt"),cert=join(root,"server-cert.pem"),key=join(root,"server-key.pem");
    const ca=await readFile(join(fixtures,"root-ca.pem"),"utf8");
    const wrongCa=await readFile(join(fixtures,"wrong-root-ca.pem"),"utf8");
    await writeFile(acl,"11".repeat(32)+" orders r\n"+"22".repeat(32)+" orders w\n",{mode:0o600});
    await writeFile(cert,await readFile(join(fixtures,"server-cert.pem")));
    await writeFile(key,await readFile(join(fixtures,"server-key.pem")),{mode:0o600});
    const store=join(root,"store");
    server=await startNative(process.env.RYSTEMDB_BINARY,store,acl,cert,key);
    const connect=(token,serverName="localhost",trust=ca)=>RystemLocalClient.connectTls({
      host:"127.0.0.1",port:server.port,serverName,ca:trust,token,
    });
    try {
      await assert.rejects(connect(tokenR,"not-localhost.test"),/cert|name|alt|hostname|verify|TLS/i);
      await assert.rejects(connect(tokenR,"localhost",wrongCa),/cert|verify|authority|issuer|TLS/i);
      await assert.rejects(connect(new Uint8Array(32)),error=>error.status===6);
      await assert.rejects(
        RystemLocalClient.connect("127.0.0.1",server.port,tokenW),
        /closed|socket|timeout|connection|ECONNRESET|unsupported|frame/i
      );
      const writer=await connect(tokenW),reader=await connect(tokenR);
      try {
        await writer.ping();
        await writer.put("orders","🦊",document());
        await writer.put("orders","\uFEFFid",document());
        assert.deepEqual(await reader.get("orders","🦊"),document());
        await assert.rejects(writer.get("orders","🦊"),e=>e.status===6);
        await assert.rejects(reader.delete("orders","🦊"),e=>e.status===6);
        await assert.rejects(writer.put("other","id",document()),e=>e.status===6);
        await Promise.all(Array.from({length:6},async(_,i)=>{
          await writer.put("orders","n"+i,document());
          assert.deepEqual(await reader.get("orders","n"+i),document());
        }));
        assert.equal(await writer.delete("orders","🦊"),true);
        assert.equal(await reader.get("orders","🦊"),null);
      } finally {await writer.close();await reader.close();}
    } finally {await stopNative(server);server=undefined;}
    server=await startNative(process.env.RYSTEMDB_BINARY,store,acl,cert,key);
    const reader=await RystemLocalClient.connectTls({
      host:"127.0.0.1",port:server.port,serverName:"localhost",ca,token:tokenR,
    });
    try {
      assert.deepEqual(await reader.get("orders","\uFEFFid"),document());
    } finally {await reader.close();}
  } finally {
    if(server)await stopNative(server);
    await rm(root,{recursive:true,force:true});
  }
});
test("TDD RED #52: Node TLS API refuses remote targets and malformed credentials before connect",async()=>{
  await assert.rejects(RystemLocalClient.connectTls({
    host:"192.0.2.3",port:8790,serverName:"localhost",ca:"abc",token:tokenR,
  }),/loopback|localhost|local only/i);
  await assert.rejects(RystemLocalClient.connectTls({
    host:"127.0.0.1",port:8790,serverName:"localhost",ca:"abc",token:new Uint8Array(31),
  }),/32/i);
});
