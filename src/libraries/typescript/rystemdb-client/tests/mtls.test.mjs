import test from "node:test";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { mkdtemp, rm, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { RystemLocalClient } from "../src/client.ts";
import { RdbReference } from "../src/rdb.ts";

const serverFixtures = fileURLToPath(
  new URL("../../../../rystemdb-cli/tests/fixtures/tls/", import.meta.url));
const token = Uint8Array.from({length:32}, () => 0x11);
const doc = () => new Map([
  ["count", -9223372036854775808n],
  ["binary", Uint8Array.from([0, 128, 255])],
  ["reference", new RdbReference("users", "u42")],
  ["items", [null, "Roma 🏛"]],
]);

// Every private CA/client key is generated solely under a fresh test temp
// directory. OpenSSL output is never logged, including after failures.
function openssl(...args) {
  const result = spawnSync("openssl", args, {encoding:"utf8", timeout:20000});
  assert.equal(result.error, undefined, "OpenSSL must be installed for ephemeral mTLS tests");
  assert.equal(result.status, 0, "Ephemeral OpenSSL fixture generation failed");
}
async function createCa(root, name) {
  const config=join(root,name+"-ca.cnf");
  await writeFile(config,
    "[req]\ndistinguished_name=dn\nprompt=no\nx509_extensions=ca\n" +
    "[dn]\nCN=TEST-ONLY-"+name+"\n" +
    "[ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n");
  const cert=join(root,name+"-ca.pem");
  const key=join(root,name+"-ca.key");
  openssl("req","-x509","-newkey","rsa:2048","-nodes","-days","3",
    "-keyout",key,"-out",cert,"-config",config);
  return {cert,key};
}
async function createIdentity(root, name, ca, uri, eku="clientAuth") {
  const key=join(root,name+".key");
  const req=join(root,name+".csr");
  const cert=join(root,name+".pem");
  const config=join(root,name+".cnf");
  await writeFile(config,
    "[identity]\nbasicConstraints=critical,CA:FALSE\n" +
    "keyUsage=critical,digitalSignature\nextendedKeyUsage="+eku+"\n" +
    "subjectAltName=URI:"+uri+"\n");
  openssl("req","-new","-newkey","rsa:2048","-nodes","-keyout",key,
    "-out",req,"-subj","/CN=DO-NOT-TRUST-CN");
  openssl("x509","-req","-in",req,"-CA",ca.cert,"-CAkey",ca.key,
    "-CAcreateserial","-out",cert,"-days","2","-extfile",config,
    "-extensions","identity");
  return {cert:await readFile(cert),key:await readFile(key)};
}
async function fixture() {
  const root=await mkdtemp(join(tmpdir(),"rystem-node-mtls-"));
  try {
    const ca=await createCa(root,"client");
    const wrongCa=await createCa(root,"untrusted");
    const writer=await createIdentity(root,"writer",ca,
      "spiffe://rystemdb.local/client/writer");
    const reader=await createIdentity(root,"reader",ca,
      "spiffe://rystemdb.local/client/reader");
    const stranger=await createIdentity(root,"stranger",ca,
      "spiffe://rystemdb.local/client/stranger");
    const wrongEku=await createIdentity(root,"wrong-eku",ca,
      "spiffe://rystemdb.local/client/writer","serverAuth");
    const wrongIssuer=await createIdentity(root,"wrong-issuer",wrongCa,
      "spiffe://rystemdb.local/client/writer");
    const serverCert=join(root,"server-cert.pem"),serverKey=join(root,"server-key.pem");
    const auth=join(root,"token.acl"),principals=join(root,"principal.acl");
    await writeFile(serverCert,await readFile(join(serverFixtures,"server-cert.pem")));
    await writeFile(serverKey,await readFile(join(serverFixtures,"server-key.pem")),{mode:0o600});
    await writeFile(auth,"11".repeat(32)+" orders rw\n"+"22".repeat(32)+" inventory rw\n",
      {mode:0o600});
    await writeFile(principals,
      "rystem-mtls-principal-acl-v1\n" +
      "spiffe://rystemdb.local/client/writer orders rw\n" +
      "spiffe://rystemdb.local/client/reader orders r\n",{mode:0o600});
    return {root,ca,writer,reader,stranger,wrongEku,wrongIssuer,serverCert,
      serverKey,auth,principals,serverCa:await readFile(join(serverFixtures,"root-ca.pem"))};
  } catch (e) {
    await rm(root,{recursive:true,force:true});
    throw e;
  }
}
async function startNative(binary, f) {
  const args=["rpc-serve","--data",join(f.root,"store"),"--listen","127.0.0.1:0",
    "--auth-file",f.auth,"--tls-cert",f.serverCert,"--tls-key",f.serverKey,
    "--mtls-ca",f.ca.cert,"--mtls-policy",f.principals];
  const child=spawn(binary,args,{stdio:["ignore","pipe","pipe"]});
  try {
    const rl=createInterface({input:child.stdout});
    const [line]=await Promise.race([
      once(rl,"line"),
      once(child,"exit").then(([code])=>{throw new Error("mTLS server exited "+code);}),
      new Promise((_,reject)=>{
        const timer=setTimeout(()=>reject(new Error("mTLS listener startup timed out")),20000);
        timer.unref();
      }),
    ]);
    rl.close();
    assert.match(line,/LOOPBACK RPC listening on 127\.0\.0\.1:\d+$/);
    return {child,port:Number(line.match(/:(\d+)$/)[1])};
  } catch (error) {
    child.kill();
    throw error;
  }
}
async function stopNative(server) {
  if(!server.child.killed)server.child.kill();
  if(server.child.exitCode === null) {
    await Promise.race([
      once(server.child,"exit").catch(()=>{}),
      new Promise(resolve=>{const timer=setTimeout(resolve,2500);timer.unref();}),
    ]);
  }
}

test("TDD RED #68: real Rust mTLS Node certificate/token principal ACL intersection",{
  skip:!process.env.RYSTEMDB_BINARY,timeout:90000,
},async()=>{
  const f=await fixture();
  let server;
  try {
    server=await startNative(process.env.RYSTEMDB_BINARY,f);
    const base={host:"127.0.0.1",port:server.port,serverName:"localhost",
      ca:f.serverCa,token};
    const connect=(identity,other={})=>RystemLocalClient.connectTls({
      ...base,cert:identity.cert,key:identity.key,...other,
    });

    // A token alone never replaces mandatory X.509 client authentication.
    await assert.rejects(RystemLocalClient.connectTls(base));
    // Neither a wrong issuer nor a valid issuer with wrong EKU/unknown SAN
    // may complete the RSWP HELLO and perform any document operation.
    for(const identity of [f.wrongIssuer,f.wrongEku,f.stranger]) {
      await assert.rejects(connect(identity),
        "Untrusted/unknown/non-clientAuth identity must never connect");
    }
    // The independently verified server CA and hostname still apply.
    await assert.rejects(connect(f.writer,{serverName:"not-localhost.test"}));
    await assert.rejects(connect(f.writer,{ca:await readFile(
      join(serverFixtures,"wrong-root-ca.pem"))}));
    // A verified certificate alone cannot authorize an invalid bearer token.
    await assert.rejects(connect(f.writer,{token:new Uint8Array(32)}));

    const writer=await connect(f.writer);
    const reader=await connect(f.reader);
    try {
      await writer.ping();
      await writer.put("orders","secure",doc());
      assert.deepEqual(await writer.get("orders","secure"),doc());
      assert.deepEqual(await reader.get("orders","secure"),doc());
      await assert.rejects(reader.put("orders","deny",doc()),e=>e.status===6);
      await assert.rejects(reader.delete("orders","secure"),e=>e.status===6);
      await assert.rejects(writer.put("inventory","hidden",doc()),e=>e.status===6);
      assert.equal(await writer.delete("orders","secure"),true);
      assert.equal(await reader.get("orders","secure"),null);
    } finally {
      await writer.close();
      await reader.close();
    }
  } finally {
    if(server)await stopNative(server);
    await rm(f.root,{recursive:true,force:true});
  }
});

test("TDD RED #68: Node mTLS rejects incomplete and oversized PEM before socket use",
async()=>{
  const base={host:"127.0.0.1",port:1,serverName:"localhost",ca:"fake CA",token};
  await assert.rejects(RystemLocalClient.connectTls({
    ...base,cert:"-----BEGIN CERTIFICATE-----\nabc\n-----END CERTIFICATE-----",
  }),/client|private|key|pair/i);
  await assert.rejects(RystemLocalClient.connectTls({
    ...base,key:"-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----",
  }),/client|cert|pair/i);
  await assert.rejects(RystemLocalClient.connectTls({
    ...base,cert:"x".repeat(65537),key:"y",
  }),/client|PEM|size|limit|certificate/i);
  await assert.rejects(RystemLocalClient.connectTls({
    ...base,cert:"not PEM",key:"not PEM",
  }),/client|PEM|key|certificate/i);
  await assert.rejects(RystemLocalClient.connectTls({
    ...base,host:"192.0.2.10",cert:"x",key:"y",
  }),/loopback/i);
});
