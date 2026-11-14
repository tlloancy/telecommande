// WebRTC : la vidéo arrive en piste, la souris part sur le canal non fiable, le reste sur le canal fiable.
export type ScreenInfo = { name: string; label: string; x: number; y: number; w: number; h: number };
export type Layout = { w: number; h: number; screens: ScreenInfo[] };

export type Ev =
  | { t: "move"; dx: number; dy: number }
  | { t: "abs"; x: number; y: number }
  | { t: "button"; b: number; down: boolean }
  | { t: "scroll"; dx: number; dy: number }
  | { t: "key"; code: number; down: boolean }
  | { t: "text"; s: string }
  | { t: "hello" }
  | { t: "focus"; name: string };

type Sig =
  | { kind: "offer"; sdp: string }
  | { kind: "ice"; candidate?: string; sdpMLineIndex?: number | null }
  | { kind: "error"; message?: string };

export class Link {
  private ws?: WebSocket;
  private pc?: RTCPeerConnection;
  private move?: RTCDataChannel;
  private ctrl?: RTCDataChannel;
  private gen = 0;
  private token = "";
  /// Échecs d'ouverture consécutifs : un jeton refusé (401) ferme le WebSocket avant `open`.
  private failedOpens = 0;
  private remoteReady = false;
  private iceQueue: RTCIceCandidateInit[] = [];
  onStatus: (text: string, live: boolean) => void = () => {};
  onStream: (stream: MediaStream | null) => void = () => {};
  onLayout: (layout: Layout) => void = () => {};

  connect(token: string) {
    this.gen += 1;
    const gen = this.gen;
    this.token = token;
    this.teardown();
    this.remoteReady = false;
    this.iceQueue = [];
    this.onStatus("Connexion…", false);
    this.onStream(null);

    const ws = new WebSocket(`wss://${location.host}/ws?token=${encodeURIComponent(token)}`);
    this.ws = ws;
    const pc = new RTCPeerConnection({ bundlePolicy: "max-bundle", iceServers: [] });
    this.pc = pc;

    pc.ontrack = (ev) => {
      const stream = ev.streams[0] ?? new MediaStream([ev.track]);
      this.onStream(stream);
    };
    pc.ondatachannel = (ev) => {
      const ch = ev.channel;
      if (ch.label === "move") this.move = ch;
      else if (ch.label === "ctrl") {
        this.ctrl = ch;
        ch.onmessage = (msg) => {
          if (typeof msg.data !== "string") return;
          try {
            const data = JSON.parse(msg.data) as { t?: string };
            if (data.t === "layout") this.onLayout(data as Layout);
          } catch { /* message d'entrée, pas une géométrie */ }
        };
        const ask = () => { if (ch.readyState === "open") ch.send(JSON.stringify({ t: "hello" })); };
        ch.onopen = ask;
        ask();
      }
    };
    // Le WebSocket peut rester « ouvert » alors que la liaison vidéo est morte (changement de
    // pièce = changement de borne Wi-Fi, donc d'adresse). On relance la connexion nous-mêmes.
    let dropTimer: number | undefined;
    pc.onconnectionstatechange = () => {
      if (gen !== this.gen) return;
      window.clearTimeout(dropTimer);
      if (pc.connectionState === "failed") this.reconnect(gen);
      else if (pc.connectionState === "disconnected") {
        this.onStatus("Signal faible, reconnexion…", false);
        dropTimer = window.setTimeout(() => this.reconnect(gen), 4000);
      }
    };
    pc.onicecandidate = (ev) => {
      if (!ev.candidate || ws.readyState !== WebSocket.OPEN) return;
      ws.send(JSON.stringify({
        kind: "ice",
        candidate: ev.candidate.candidate,
        sdpMLineIndex: ev.candidate.sdpMLineIndex,
      }));
    };

    ws.onmessage = (ev) => {
      if (gen !== this.gen || typeof ev.data !== "string") return;
      void this.onSignal(pc, ws, ev.data);
    };
    let opened = false;
    ws.onopen = () => { opened = true; this.failedOpens = 0; };
    ws.onclose = () => {
      if (gen !== this.gen) return;
      if (!opened) this.failedOpens += 1;
      const stuck = this.failedOpens >= 3;
      this.onStream(null);
      this.onStatus(stuck
        ? "Bureau éteint ou téléphone non reconnu : rescanne le QR affiché sur l’ordinateur."
        : "Connexion perdue, nouvelle tentative…", false);
      window.setTimeout(() => {
        if (gen === this.gen) this.connect(token);
      }, stuck ? 5000 : 1500);
    };
    ws.onerror = () => {
      if (gen === this.gen) this.onStatus("Connexion…", false);
    };
  }

  private reconnect(gen: number) {
    if (gen === this.gen && this.token) this.connect(this.token);
  }

  /// À appeler quand le téléphone se réveille ou retrouve le réseau : si la liaison n'est pas
  /// saine, on repart tout de suite au lieu d'attendre un délai.
  resume() {
    if (!this.token) return;
    const healthy = this.ws?.readyState === WebSocket.OPEN && this.pc?.connectionState === "connected";
    if (healthy) return;
    this.failedOpens = 0;
    this.connect(this.token);
  }

  send(ev: Ev) {
    const ch = ev.t === "move" ? this.move : this.ctrl;
    if (ch?.readyState === "open") ch.send(JSON.stringify(ev));
  }

  private teardown() {
    this.ws?.close();
    this.pc?.close();
    this.ws = undefined;
    this.pc = undefined;
    this.move = undefined;
    this.ctrl = undefined;
  }

  private async onSignal(pc: RTCPeerConnection, ws: WebSocket, raw: string) {
    let msg: Sig;
    try { msg = JSON.parse(raw) as Sig; } catch { return; }
    if (msg.kind === "error") {
      this.onStatus(msg.message || "La connexion a échoué.", false);
      return;
    }
    try {
      if (msg.kind === "offer") {
        await pc.setRemoteDescription({ type: "offer", sdp: msg.sdp });
        this.remoteReady = true;
        const answer = await pc.createAnswer();
        await pc.setLocalDescription(answer);
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ kind: "answer", sdp: answer.sdp }));
        }
        const queued = this.iceQueue.splice(0);
        for (const c of queued) await pc.addIceCandidate(c);
      } else if (msg.kind === "ice" && msg.candidate && msg.sdpMLineIndex != null) {
        const init = { candidate: msg.candidate, sdpMLineIndex: msg.sdpMLineIndex };
        if (!this.remoteReady) this.iceQueue.push(init);
        else await pc.addIceCandidate(init);
      }
    } catch (err) {
      console.error(err);
      this.onStatus("La vidéo n'a pas pu démarrer.", false);
    }
  }
}
