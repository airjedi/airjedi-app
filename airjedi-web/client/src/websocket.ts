import { ServerMessage, ClientMessage } from "./types";

export class WebSocketClient {
  private ws: WebSocket | null = null;
  private url: string = "";
  private reconnectTimer: number | null = null;
  onMessage: ((msg: ServerMessage) => void) | null = null;
  onConnectionChange: ((connected: boolean) => void) | null = null;

  connect(url: string): void {
    this.url = url;
    this.doConnect();
  }

  send(msg: ClientMessage): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg));
    }
  }

  private doConnect(): void {
    if (this.ws) {
      this.ws.close();
    }

    this.ws = new WebSocket(this.url);

    this.ws.onopen = () => {
      console.log("WebSocket connected");
      this.onConnectionChange?.(true);
      if (this.reconnectTimer !== null) {
        clearTimeout(this.reconnectTimer);
        this.reconnectTimer = null;
      }
    };

    this.ws.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data) as ServerMessage;
        this.onMessage?.(msg);
      } catch (e) {
        console.error("Failed to parse WebSocket message:", e);
      }
    };

    this.ws.onclose = () => {
      console.log("WebSocket disconnected, reconnecting in 2s");
      this.onConnectionChange?.(false);
      this.scheduleReconnect();
    };

    this.ws.onerror = () => {
      this.ws?.close();
    };
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimer === null) {
      this.reconnectTimer = window.setTimeout(() => {
        this.reconnectTimer = null;
        this.doConnect();
      }, 2000);
    }
  }
}
