import test from "node:test";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { createInterface } from "node:readline";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { RystemLocalClient } from "../src/client.ts";
import { encodeRdb, RdbReference } from "../src/rdb.ts";

async function startNative(binary, data, authFile = undefined) {
  // The child owns its :0 port atomically, avoiding race with parallel runners.
  const args=["rpc-serve","--data",data,"--listen","127.0.0.1:0"];
  if(authFile) args.push("--auth-file",authFile);
  const child=spawn(binary, args,{
    stdio:["ignore","pipe","pipe"],
  });
  try {
    const rl=createInterface({input:child.stdout});
    const [banner]=await Promise.race([
      once(rl,"line"),
      new Promise((_,reject)=>{
        const t=setTimeout(()=>reject(new Error("Rust RPC banner timed out")),20000);
        t.unref();
      }),
      once(child,"exit").then(([code])=>{throw new Error("Rust server exited: "+code);})
    ]);
    rl.close();
    assert.match(banner,/LOOPBACK RPC listening on 127\.0\.0\.1:\d+$/);
    const port=Number(banner.match(/:(\d+)$/)[1]);
    return { child, port };
  } catch(error) {
    child.kill();
    throw error;
  }
}
async function stopNative(server) {
  if(!server.child.killed) server.child.kill();
  await Promise.race([
    once(server.child,"exit").catch(()=>{}),
    new Promise(resolve=>{const t=setTimeout(resolve,2500);t.unref();})
  ]);
}

function nestedDoc() {
  return new Map([
    ["array",[null,-9223372036854775808n,true]],
    ["binary",Uint8Array.from([0,255,128])],
    ["reference",new RdbReference("users","u42")],
    ["unicode","Roma 🏛"],
  ]);
}
test("TDD RED: Node client forbids network addresses without TLS", async () => {
  await assert.rejects(
    RystemLocalClient.connect("192.0.2.10", 8790),
    /loopback|localhost|local only/i
  );
});

test("TDD RED: Node real Rust process HELLO PING nested PUT GET DELETE and restart", {
  skip: !process.env.RYSTEMDB_BINARY,
  timeout: 60000,
}, async () => {
  const root=await mkdtemp(join(tmpdir(),"rystem-ts-live-"));
  try {
    const binary=process.env.RYSTEMDB_BINARY;
    let server=await startNative(binary,root);
    try {
      const client=await RystemLocalClient.connect("127.0.0.1",server.port);
      try {
        await client.ping();
        const doc=nestedDoc();
        await client.put("orders","🦊",doc);
        await client.put("orders","\uFEFFid",doc);
        // A real second native CLI process must not bypass the RPC owner's
        // exclusive on-disk writer lock (Linux/macOS/Windows).
        const contender = spawnSync(binary,[
          "document","delete","--data",root,
          "--collection","orders","--id","🦊"
        ],{timeout:15000,encoding:"utf8"});
        assert.equal(contender.error,undefined,
          "Failed spawning competing writer process: "+contender.error);
        assert.notEqual(contender.status,0,
          "RPC server must prevent a separate native CLI writer");
        assert.deepEqual(await client.get("orders","🦊"),doc);
        assert.equal(await client.get("orders","absent"),null);
        await Promise.all(Array.from({length:8},async (_,n)=>{
          await client.put("orders","n"+n,doc);
          assert.deepEqual(await client.get("orders","n"+n),doc);
        }));
        assert.equal(await client.delete("orders","🦊"),true);
        assert.equal(await client.delete("orders","🦊"),false);
        assert.equal(await client.get("orders","🦊"),null);
        await assert.rejects(client.put("orders","",doc),/id/i);
      } finally { await client.close(); }
    } finally { await stopNative(server); }

    server=await startNative(binary,root);
    try {
      const client=await RystemLocalClient.connect("127.0.0.1",server.port);
      try {
        const got=await client.get("orders","\uFEFFid");
        assert.deepEqual(got,nestedDoc());
        assert.deepEqual(encodeRdb(got),encodeRdb(nestedDoc()));
        assert.equal(await client.delete("orders","\uFEFFid"),true);
      } finally { await client.close(); }
    } finally { await stopNative(server); }
  } finally { await rm(root,{recursive:true,force:true}); }
});


test("TDD RED: Node authenticated HELLO and collection-specific read/write permissions", {
  skip: !process.env.RYSTEMDB_BINARY,
  timeout: 60000,
}, async () => {
  const root=await mkdtemp(join(tmpdir(),"rystem-ts-auth-"));
  const acl=join(root,"policy.txt");
  await writeFile(acl,
    "11".repeat(32)+" orders r\n"+
    "22".repeat(32)+" orders w\n",{mode:0o600});
  let server;
  try {
    server=await startNative(process.env.RYSTEMDB_BINARY,join(root,"store"),acl);
    await assert.rejects(RystemLocalClient.connect("127.0.0.1",server.port),
      error => error.status === 6);
    await assert.rejects(RystemLocalClient.connect("127.0.0.1",server.port,new Uint8Array(32)),
      error => error.status === 6);
    const writer=await RystemLocalClient.connect("127.0.0.1",server.port,
      Uint8Array.from({length:32},()=>0x22));
    const reader=await RystemLocalClient.connect("127.0.0.1",server.port,
      Uint8Array.from({length:32},()=>0x11));
    try {
      await writer.ping();
      await writer.put("orders","private",nestedDoc());
      await assert.rejects(writer.get("orders","private"), error=>error.status===6);
      assert.deepEqual(await reader.get("orders","private"),nestedDoc());
      await assert.rejects(reader.put("orders","private",nestedDoc()),
        error=>error.status===6);
      await assert.rejects(writer.put("unknown","private",nestedDoc()),
        error=>error.status===6);
      assert.equal(await writer.delete("orders","private"),true);
    } finally {
      await reader.close(); await writer.close();
    }
  } finally {
    if(server) await stopNative(server);
    await rm(root,{recursive:true,force:true});
  }
});
