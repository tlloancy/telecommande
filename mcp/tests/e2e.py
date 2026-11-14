import re, subprocess, json, os, sys, time, base64, struct, zlib, shutil
BIN = sys.argv[1]
home = "/tmp/e2e_home"; shutil.rmtree(home, ignore_errors=True); os.makedirs(home)
env = dict(os.environ, HOME=home, XDG_CONFIG_HOME=home+"/c", XDG_STATE_HOME=home+"/s", DISPLAY=":99", TELECOMMANDE_MCP_ASSUME_YES="1")
xvfb = subprocess.Popen(["Xvfb", ":99", "-screen", "0", "1280x800x24"], stderr=subprocess.DEVNULL); time.sleep(1.5)
fails = []
def check(name, cond, extra=""):
    print(("PASS " if cond else "FAIL ") + name, extra)
    if not cond: fails.append(name)
def start(extra_env=None):
    e = dict(env); e.update(extra_env or {})
    p = subprocess.Popen([BIN], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=e, text=True)
    return p
nid = [0]
def rpc(p, method, params=None):
    nid[0] += 1
    p.stdin.write(json.dumps({"jsonrpc":"2.0","id":nid[0],"method":method,"params":params or {}})+"\n"); p.stdin.flush()
    return json.loads(p.stdout.readline())
def call(p, name, args=None):
    r = rpc(p, "tools/call", {"name":name,"arguments":args or {}})
    return r["result"]
p = start()
r = rpc(p, "initialize", {"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}})
check("initialize", r["result"]["serverInfo"]["name"]=="telecommande-mcp" and r["result"]["protocolVersion"]=="2025-06-18")
p.stdin.write(json.dumps({"jsonrpc":"2.0","method":"notifications/initialized"})+"\n"); p.stdin.flush()
names = [t["name"] for t in rpc(p,"tools/list")["result"]["tools"]]
check("tools/list", names==["list_screens","screenshot","mouse_move","click","drag","scroll","type_text","key"], names)
check("ping", rpc(p,"ping")["result"]=={})
check("unknown method", rpc(p,"nope")["error"]["code"]==-32601)
res = call(p,"list_screens"); check("list_screens", not res.get("isError") and "width" in res["content"][0]["text"], res["content"][0]["text"][:80].replace("\n"," "))

# fenêtre rouge unie pour vérifier la capture
xt = subprocess.Popen(["xterm","-geometry","40x10+100+100","-bg","#ff0000","-e","sh","-c","cat > /tmp/e2e_typed.txt"], env=env)
time.sleep(1.5)
res = call(p,"screenshot",{"max_dim":640})
img = [c for c in res["content"] if c["type"]=="image"][0]
png = base64.b64decode(img["data"])
check("screenshot is PNG", png[:8]==b"\x89PNG\r\n\x1a\n" and img["mimeType"]=="image/png")
w,h = struct.unpack(">II", png[16:24]); check("screenshot scaled to max_dim", max(w,h)==640 and (w,h)==(640,400), (w,h))
open("/tmp/e2e_shot.png","wb").write(png)
# décodage pour trouver du rouge
import io
try:
    from PIL import Image
    im = Image.open(io.BytesIO(png)).convert("RGB"); reds = sum(1 for px in im.getdata() if px[0]>240 and px[1]<10 and px[2]<10)
    check("capture shows the red xterm", reds>500, reds)
except ImportError:
    print("SKIP pillow absent")
print(res["content"][0]["text"])

# clic au milieu de la fenêtre xterm : image 640 = bureau/2 ; fenêtre vers (100..~400, 100..) bureau => image (100,100)
res = call(p,"click",{"x":100,"y":80}); check("click ok", not res.get("isError"), res["content"][0]["text"])
loc = subprocess.check_output(["xdotool","getmouselocation"],env=env,text=True)
check("pointer moved to desktop (200,160)", loc.startswith("x:200 y:160"), loc.strip())
check("mouse_move", not call(p,"mouse_move",{"x":50,"y":50}).get("isError"))
loc = subprocess.check_output(["xdotool","getmouselocation"],env=env,text=True); check("mouse_move position", loc.startswith("x:100 y:100"), loc.strip())
call(p,"mouse_move",{"x":100,"y":80})
res = call(p,"type_text",{"text":"bonjour é"}); check("type_text reports skipped chars", res.get("isError") and "é" in res["content"][0]["text"] and "8 caractères tapés sur 9" in res["content"][0]["text"], res["content"][0]["text"])
res = call(p,"key",{"key":"Enter"}); check("key Enter", not res.get("isError"))
res = call(p,"key",{"key":"ctrl+d"}); check("key ctrl+d", not res.get("isError"), res["content"][0]["text"])
time.sleep(1)
typed = open("/tmp/e2e_typed.txt").read() if os.path.exists("/tmp/e2e_typed.txt") else ""
print("typed:", repr(typed))
check("keystrokes reached the window", typed.startswith("bonjour") , repr(typed))
check("scroll", not call(p,"scroll",{"dy":3,"dx":-1}).get("isError"))
check("drag", not call(p,"drag",{"from_x":10,"from_y":10,"to_x":60,"to_y":60}).get("isError"))
check("bad key rejected", call(p,"key",{"key":"hyper+x"}).get("isError") is True)
check("unknown tool rejected", call(p,"rm_rf").get("isError") is True)
check("click needs coords", call(p,"click",{}).get("isError") is True)

# arrêt d'urgence
subprocess.check_call([BIN,"stop"],env=env,stdout=subprocess.DEVNULL)
res = call(p,"click",{"x":1,"y":1}); check("emergency stop blocks click", res.get("isError") and "Arrêt d'urgence" in res["content"][0]["text"])
res = call(p,"screenshot"); check("emergency stop blocks screenshot", res.get("isError") is True)
res = call(p,"list_screens"); check("emergency stop blocks list_screens too", res.get("isError") and "Arrêt d'urgence" in res["content"][0]["text"])
st = subprocess.check_output([BIN,"status"],env=env,text=True); check("status ACTIF", "ACTIF" in st)
subprocess.check_call([BIN,"resume"],env=env,stdout=subprocess.DEVNULL)
check("resume re-enables", not call(p,"mouse_move",{"x":5,"y":5}).get("isError"))
# arrêt d'urgence en pleine saisie longue
call(p,"mouse_move",{"x":100,"y":80})
import threading
def later():
    time.sleep(0.4); subprocess.check_call([BIN,"stop"],env=env,stdout=subprocess.DEVNULL)
t=threading.Thread(target=later); t.start()
t0=time.time(); res = call(p,"type_text",{"text":"a"*5000}); t.join()
m=re.search(r"(\d+) caractères tapés sur 5000", res["content"][0]["text"]) if res.get("isError") else None
check("stop interrupts a long type_text", bool(m) and 0 < int(m.group(1)) < 5000, res["content"][0]["text"])
subprocess.check_call([BIN,"resume"],env=env,stdout=subprocess.DEVNULL)
p.stdin.close(); p.wait(timeout=5)

# refus d'accord
p2 = start({"TELECOMMANDE_MCP_ASSUME_YES":"0","TELECOMMANDE_MCP_APPROVE_CMD":"exit 1"})
rpc(p2,"initialize",{"protocolVersion":"2025-06-18"})
res = call(p2,"click",{"x":1,"y":1}); check("denied consent blocks", res.get("isError") and "refusé" in res["content"][0]["text"], res["content"][0]["text"])
res = call(p2,"key",{"key":"Enter"}); check("denial remembered briefly", res.get("isError") is True)
p2.stdin.close(); p2.wait(timeout=5)
# accord via commande
p3 = start({"TELECOMMANDE_MCP_ASSUME_YES":"0","TELECOMMANDE_MCP_APPROVE_CMD":"exit 0"})
rpc(p3,"initialize",{"protocolVersion":"2025-06-18"})
check("approve cmd allows", not call(p3,"mouse_move",{"x":5,"y":5}).get("isError"))
p3.stdin.close(); p3.wait(timeout=5)
# sans outil de confirmation
p4 = start({"TELECOMMANDE_MCP_ASSUME_YES":"0","PATH":"/nonexistent"})
rpc(p4,"initialize",{"protocolVersion":"2025-06-18"})
res = call(p4,"mouse_move",{"x":5,"y":5}); check("no dialog tool => denied", res.get("isError") and "confirmation" in res["content"][0]["text"], res["content"][0]["text"])
p4.stdin.close(); p4.wait(timeout=5)

# journal
logf = home+"/s/telecommande/mcp-actions.log"
log = open(logf).read(); lines=[json.loads(l) for l in log.splitlines()]
check("journal has entries", len(lines)>10, len(lines))
check("journal never contains typed text", "bonjour" not in log)
tt=[l for l in lines if l["tool"]=="type_text"][0]; check("type_text logged by length", tt["detail"]=={"chars":9}, tt)
check("journal records refusals", any(l["outcome"].startswith("refusé") for l in lines))
sd=os.stat(home+"/s/telecommande").st_mode & 0o777; check("journal dir perms 700", sd==0o700, oct(sd))
check("journal perms 600", oct(os.stat(logf).st_mode & 0o777)=="0o600", oct(os.stat(logf).st_mode & 0o777))
xt.terminate(); xvfb.terminate()
print("\nFAILS:", fails)
sys.exit(1 if fails else 0)
