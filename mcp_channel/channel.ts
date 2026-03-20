#!/usr/bin/env bun
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  ListToolsRequestSchema,
  CallToolRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";
import { connect } from "net";
import { createHash } from "crypto";
import { statSync, existsSync } from "fs";
import { resolve, join, dirname } from "path";

// -- Socket path computation (mirrors Rust's compute_socket_path) --

function findRepoRoot(startDir: string): string {
  let dir = resolve(startDir);
  while (true) {
    try {
      statSync(join(dir, ".git"));
      return dir;
    } catch {
      const parent = dirname(dir);
      if (parent === dir) return resolve(startDir);
      dir = parent;
    }
  }
}

function computeSocketPath(dir: string): string {
  const abs = resolve(dir);
  const hash = createHash("sha256").update(abs).digest("hex").slice(0, 12);
  return "/tmp/tuicr-" + hash + ".sock";
}

// -- Types --

type Message = {
  type: string;
  [key: string]: any;
};

// -- Engine Connection --

class EngineConnection {
  private socketPath: string;
  private conn: ReturnType<typeof connect> | null = null;
  private pendingRequests = new Map<
    string,
    { resolve: (msg: Message) => void; reject: (err: Error) => void }
  >();
  private onEvent: (event: string, payload: Record<string, any>) => void;
  private lineBuffer = "";
  private reconnecting = false;
  private closed = false;

  constructor(
    socketPath: string,
    onEvent: (event: string, payload: Record<string, any>) => void,
  ) {
    this.socketPath = socketPath;
    this.onEvent = onEvent;
  }

  async connect(): Promise<void> {
    return new Promise((resolve, reject) => {
      if (!existsSync(this.socketPath)) {
        reject(new Error("Socket not found: " + this.socketPath));
        return;
      }

      this.conn = connect(this.socketPath, () => {
        // Send subscribe message
        const sub = JSON.stringify({
          type: "subscribe",
          events: ["feedback_submitted"],
        });
        this.conn!.write(sub + "\n");
      });

      this.conn.setEncoding("utf8");
      this.lineBuffer = "";
      let gotAck = false;

      this.conn.on("data", (chunk: string) => {
        this.lineBuffer += chunk;
        const lines = this.lineBuffer.split("\n");
        this.lineBuffer = lines.pop() || "";

        for (const line of lines) {
          if (!line.trim()) continue;
          try {
            const msg: Message = JSON.parse(line);
            if (!gotAck && msg.type === "subscribe_response") {
              gotAck = true;
              resolve();
              continue;
            }
            this.handleMessage(msg);
          } catch {
            // ignore malformed lines
          }
        }
      });

      this.conn.on("error", (err: Error) => {
        if (!gotAck) {
          reject(err);
        } else {
          this.scheduleReconnect();
        }
      });

      this.conn.on("close", () => {
        if (gotAck && !this.closed) {
          this.scheduleReconnect();
        }
      });
    });
  }

  private handleMessage(msg: Message) {
    if (msg.type === "event_notification") {
      this.onEvent(msg.event, msg.payload || {});
      return;
    }

    // Response to a request — match by type
    const key = msg.type;
    const pending = this.pendingRequests.get(key);
    if (pending) {
      this.pendingRequests.delete(key);
      pending.resolve(msg);
    }
  }

  async request(msg: Message): Promise<Message> {
    if (!this.conn || this.conn.destroyed) {
      throw new Error("Not connected to tuicr");
    }

    return new Promise((resolve, reject) => {
      const responseType = msg.type + "_response";
      this.pendingRequests.set(responseType, { resolve, reject });
      this.conn!.write(JSON.stringify(msg) + "\n");

      // Timeout after 30s
      setTimeout(() => {
        if (this.pendingRequests.has(responseType)) {
          this.pendingRequests.delete(responseType);
          reject(new Error("Request timed out: " + msg.type));
        }
      }, 30000);
    });
  }

  private scheduleReconnect() {
    if (this.reconnecting || this.closed) return;
    this.reconnecting = true;

    const attempt = (delay: number) => {
      if (this.closed) return;
      setTimeout(async () => {
        try {
          await this.connect();
          this.reconnecting = false;
        } catch {
          attempt(Math.min(delay * 2, 10000));
        }
      }, delay);
    };

    attempt(1000);
  }

  close() {
    this.closed = true;
    if (this.conn) {
      this.conn.destroy();
      this.conn = null;
    }
  }
}

// -- Blocking connection for get_feedback --wait --

async function blockingGetFeedback(socketPath: string): Promise<Message> {
  return new Promise((resolve, reject) => {
    const conn = connect(socketPath, () => {
      const msg = JSON.stringify({ type: "poll_feedback", wait: true });
      conn.write(msg + "\n");
    });

    conn.setEncoding("utf8");
    let buf = "";

    conn.on("data", (chunk: string) => {
      buf += chunk;
      const lines = buf.split("\n");
      buf = lines.pop() || "";

      for (const line of lines) {
        if (!line.trim()) continue;
        try {
          const msg = JSON.parse(line);
          resolve(msg);
          conn.destroy();
        } catch {
          // ignore
        }
      }
    });

    conn.on("error", (err: Error) => reject(err));
    conn.on("close", () => reject(new Error("Connection closed")));
  });
}

// -- Main --

const cwd = process.cwd();
const repoRoot = findRepoRoot(cwd);
const socketPath = computeSocketPath(repoRoot);

// Create MCP server with channel capability
const mcp = new Server(
  { name: "tuicr", version: "1.0.0" },
  {
    capabilities: {
      experimental: { "claude/channel": {} },
      tools: {},
    },
    instructions: [
      "Events from the tuicr channel arrive as <channel source=\"tuicr\" event=\"...\">.",
      "These are review events from your human reviewer who is watching your code changes using tuicr.",
      "",
      "When you receive a feedback_submitted event, use the get_feedback tool to retrieve the detailed review feedback.",
      "You can check the current review status at any time using the review_status tool.",
    ].join("\n"),
  },
);

// Engine connection with event handler that pushes channel notifications
const engine = new EngineConnection(socketPath, (event, payload) => {
  switch (event) {
    case "feedback_submitted":
      mcp
        .notification({
          method: "notifications/claude/channel",
          params: {
            content:
              payload.message || "Your reviewer has submitted feedback. Use the get_feedback tool to retrieve it.",
            meta: { event: "feedback_submitted" },
          },
        })
        .catch(() => {});
      break;
  }
});

// -- Tools --

mcp.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: [
    {
      name: "review_status",
      description:
        "Check the current status of the code review (comment count, files reviewed)",
      inputSchema: {
        type: "object" as const,
        properties: {},
      },
    },
    {
      name: "get_feedback",
      description:
        "Retrieve review feedback from your reviewer. Use wait=true to block until feedback is available.",
      inputSchema: {
        type: "object" as const,
        properties: {
          wait: {
            type: "boolean",
            description: "Block until feedback is available",
          },
        },
      },
    },
  ],
}));

mcp.setRequestHandler(CallToolRequestSchema, async (req) => {
  const args = (req.params.arguments || {}) as Record<string, any>;

  switch (req.params.name) {
    case "review_status": {
      try {
        const resp = await engine.request({ type: "get_review_status" });
        return {
          content: [
            { type: "text" as const, text: resp.summary || "No reviewer connected." },
          ],
        };
      } catch {
        return {
          content: [{ type: "text" as const, text: "No reviewer connected." }],
        };
      }
    }

    case "get_feedback": {
      try {
        let resp: Message;
        if (args.wait) {
          resp = await blockingGetFeedback(socketPath);
        } else {
          resp = await engine.request({ type: "poll_feedback", wait: false });
        }

        if (resp.has_feedback) {
          return {
            content: [{ type: "text" as const, text: resp.feedback }],
          };
        }
        return {
          content: [{ type: "text" as const, text: "No feedback pending." }],
        };
      } catch {
        return {
          content: [{ type: "text" as const, text: "No reviewer connected." }],
        };
      }
    }

    default:
      throw new Error(`Unknown tool: ${req.params.name}`);
  }
});

// -- Start --

async function main() {
  // Connect to tuicr engine (retry if not yet started)
  let connected = false;
  for (let i = 0; i < 5; i++) {
    try {
      await engine.connect();
      connected = true;
      break;
    } catch {
      await new Promise((r) => setTimeout(r, 2000));
    }
  }

  if (!connected) {
    console.error(
      "Warning: Could not connect to tuicr. Will retry in background.",
    );
  }

  // Connect to Claude Code via stdio
  const transport = new StdioServerTransport();
  await mcp.connect(transport);

  // Handle graceful shutdown
  process.on("SIGINT", () => {
    engine.close();
    process.exit(0);
  });
  process.on("SIGTERM", () => {
    engine.close();
    process.exit(0);
  });
}

main().catch((err) => {
  console.error("Fatal:", err);
  process.exit(1);
});
