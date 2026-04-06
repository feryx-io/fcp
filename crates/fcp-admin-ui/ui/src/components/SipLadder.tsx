import { useState, useMemo, useCallback, useEffect } from 'react'
import {
  ReactFlow,
  ReactFlowProvider,
  Background,
  Controls,
  MiniMap,
  useViewport,
  useReactFlow,
  type Node,
  type NodeProps,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import type { SipTraceMessage } from '@/api/endpoints'

interface SipLadderProps {
  messages: SipTraceMessage[]
  /** If true, auto-pan to latest message when new messages arrive */
  live?: boolean
}

// Color for SIP method/status
function getMessageColor(methodOrStatus: string): string {
  const code = parseInt(methodOrStatus, 10)
  if (!isNaN(code)) {
    if (code >= 100 && code < 200) return '#3b82f6' // blue - provisional
    if (code >= 200 && code < 300) return '#22c55e' // green - success
    if (code >= 300 && code < 400) return '#eab308' // yellow - redirect
    if (code >= 400) return '#ef4444' // red - error
  }
  // Requests
  return '#6b7280' // gray
}

// Standard SIP reason phrases
const SIP_REASON: Record<number, string> = {
  100: 'Trying', 180: 'Ringing', 181: 'Call Is Being Forwarded',
  182: 'Queued', 183: 'Session Progress', 199: 'Early Dialog Terminated',
  200: 'OK', 202: 'Accepted',
  300: 'Multiple Choices', 301: 'Moved Permanently', 302: 'Moved Temporarily',
  305: 'Use Proxy', 380: 'Alternative Service',
  400: 'Bad Request', 401: 'Unauthorized', 403: 'Forbidden', 404: 'Not Found',
  405: 'Method Not Allowed', 406: 'Not Acceptable', 407: 'Proxy Auth Required',
  408: 'Request Timeout', 410: 'Gone', 413: 'Request Entity Too Large',
  415: 'Unsupported Media Type', 416: 'Unsupported URI Scheme',
  420: 'Bad Extension', 421: 'Extension Required', 423: 'Interval Too Brief',
  480: 'Temporarily Unavailable', 481: 'Call/Transaction Does Not Exist',
  482: 'Loop Detected', 483: 'Too Many Hops', 484: 'Address Incomplete',
  485: 'Ambiguous', 486: 'Busy Here', 487: 'Request Terminated',
  488: 'Not Acceptable Here', 491: 'Request Pending', 493: 'Undecipherable',
  500: 'Server Internal Error', 501: 'Not Implemented', 502: 'Bad Gateway',
  503: 'Service Unavailable', 504: 'Server Time-out',
  505: 'Version Not Supported', 513: 'Message Too Large',
  600: 'Busy Everywhere', 603: 'Decline', 604: 'Does Not Exist Anywhere',
  606: 'Not Acceptable',
}

function getMessageLabel(msg: SipTraceMessage): string {
  const code = parseInt(msg.method_or_status, 10)
  if (!isNaN(code)) {
    const reason = SIP_REASON[code] || 'Unknown'
    return `${code} ${reason}`
  }
  return msg.method_or_status
}

interface Column {
  addr: string
  x: number
}

const COL_SPACING = 240
const COL_FIRST_X = 120
const ROW_HEIGHT = 42
const TOP_MARGIN = 40
const ARROW_Y_OFFSET = 14
const NODE_HEIGHT = 28
const HEADER_HEIGHT = 32

/**
 * Infer columns for the ladder diagram from the message set.
 * Supports multi-hop: each unique address becomes its own column.
 * Ordering: inbound-side peers (callers) → local addresses → outbound-side peers (callees).
 */
function inferColumns(messages: SipTraceMessage[]): Column[] {
  const localSet = new Set<string>()
  for (const msg of messages) {
    if (msg.direction === 'inbound') localSet.add(msg.dest_addr)
    else localSet.add(msg.source_addr)
  }

  const callers: string[] = []
  const callees: string[] = []
  const locals: string[] = []
  const seen = new Set<string>()

  const add = (addr: string, side: 'in' | 'out') => {
    if (!addr || seen.has(addr)) return
    seen.add(addr)
    if (localSet.has(addr)) locals.push(addr)
    else if (side === 'in') callers.push(addr)
    else callees.push(addr)
  }

  for (const msg of messages) {
    if (msg.direction === 'inbound') {
      add(msg.source_addr, 'in')
      add(msg.dest_addr, 'out')
    } else {
      add(msg.dest_addr, 'out')
      add(msg.source_addr, 'in')
    }
  }

  const ordered = [...callers, ...locals, ...callees]
  return ordered.map((addr, i) => ({
    addr,
    x: COL_FIRST_X + i * COL_SPACING,
  }))
}

function getArrowXs(msg: SipTraceMessage, columns: Column[]): { fromX: number; toX: number } | null {
  const sourceCol = columns.find((c) => c.addr === msg.source_addr)
  const destCol = columns.find((c) => c.addr === msg.dest_addr)
  if (!sourceCol || !destCol) return null
  return { fromX: sourceCol.x, toX: destCol.x }
}

// ─────────────────────────────────────────────────────────────────────────────
// Custom nodes

interface LifelinesData extends Record<string, unknown> {
  columns: Column[]
  height: number
  width: number
}

function LifelinesNode({ data }: NodeProps<Node<LifelinesData>>) {
  return (
    <svg
      width={data.width}
      height={data.height}
      style={{ pointerEvents: 'none', display: 'block' }}
    >
      {data.columns.map((col) => (
        <line
          key={col.addr}
          x1={col.x}
          y1={0}
          x2={col.x}
          y2={data.height}
          stroke="currentColor"
          strokeWidth={1}
          strokeDasharray="4 4"
          opacity={0.3}
          className="stroke-foreground"
        />
      ))}
    </svg>
  )
}

interface MessageNodeData extends Record<string, unknown> {
  fromX: number
  toX: number
  label: string
  color: string
  selected: boolean
  timestampLabel: string
  width: number
}

function MessageNode({ data }: NodeProps<Node<MessageNodeData>>) {
  const { fromX, toX, label, color, selected, timestampLabel, width } = data
  const arrowHead = 8
  const direction = toX > fromX ? 1 : -1
  const arrowTipX = toX
  const arrowBaseX = toX - direction * arrowHead
  const midX = (fromX + toX) / 2
  const y = ARROW_Y_OFFSET

  return (
    <div
      style={{ width, height: NODE_HEIGHT, cursor: 'pointer' }}
      className={selected ? 'bg-accent/30 rounded' : 'hover:bg-accent/10 rounded'}
    >
      <svg width={width} height={NODE_HEIGHT} className="select-none" style={{ display: 'block' }}>
        {/* Timestamp at left */}
        <text
          x={8}
          y={y + 4}
          fontSize={9}
          fontFamily="monospace"
          fill="currentColor"
          opacity={0.5}
          className="fill-foreground"
        >
          {timestampLabel}
        </text>
        {/* Hover hit area */}
        <line x1={fromX} y1={y} x2={toX} y2={y} stroke="transparent" strokeWidth={16} />
        {/* Visible line */}
        <line
          x1={fromX}
          y1={y}
          x2={toX}
          y2={y}
          stroke={color}
          strokeWidth={selected ? 2.5 : 1.5}
        />
        {/* Arrow head */}
        <polygon
          points={`${arrowTipX},${y} ${arrowBaseX},${y - arrowHead / 2} ${arrowBaseX},${y + arrowHead / 2}`}
          fill={color}
        />
        {/* Label */}
        <text
          x={midX}
          y={y - 6}
          textAnchor="middle"
          fontSize={11}
          fontFamily="monospace"
          fill={color}
          fontWeight={selected ? 700 : 400}
        >
          {label}
        </text>
      </svg>
    </div>
  )
}

// ─────────────────────────────────────────────────────────────────────────────
// Sticky headers — synced to viewport pan/zoom

function StickyHeaders({ columns }: { columns: Column[] }) {
  const { x: vx, zoom } = useViewport()

  return (
    <div
      style={{
        position: 'absolute',
        top: 0,
        left: 0,
        right: 0,
        height: HEADER_HEIGHT,
        pointerEvents: 'none',
        overflow: 'hidden',
        zIndex: 5,
      }}
      className="bg-card/90 backdrop-blur-sm border-b"
    >
      {columns.map((col) => {
        const fontSize = Math.max(9, Math.min(12 * zoom, 14))
        const displayAddr = col.addr.length > 24 ? col.addr.slice(0, 24) + '...' : col.addr
        return (
          <div
            key={col.addr}
            style={{
              position: 'absolute',
              left: col.x * zoom + vx,
              top: 8,
              transform: 'translateX(-50%)',
              fontSize,
              fontWeight: 600,
              whiteSpace: 'nowrap',
              fontFamily: 'monospace',
            }}
            className="text-foreground"
            title={col.addr}
          >
            {displayAddr}
          </div>
        )
      })}
    </div>
  )
}

const nodeTypes = {
  lifelines: LifelinesNode,
  message: MessageNode,
}

// ─────────────────────────────────────────────────────────────────────────────
// Inner component (inside ReactFlowProvider)

function SipLadderInner({ messages, live }: SipLadderProps) {
  const [selectedIdx, setSelectedIdx] = useState<number | null>(null)
  const reactFlow = useReactFlow()

  const columns = useMemo(() => inferColumns(messages), [messages])

  const baseTime = messages.length > 0 ? messages[0].timestamp : 0
  const formatRelTime = useCallback(
    (ts: number) => {
      const delta = ts - baseTime
      if (delta < 1000) return `+${delta}ms`
      return `+${(delta / 1000).toFixed(2)}s`
    },
    [baseTime],
  )

  const totalHeight = TOP_MARGIN + messages.length * ROW_HEIGHT + 30
  const totalWidth =
    columns.length > 0 ? columns[columns.length - 1].x + COL_FIRST_X : 600

  const nodes = useMemo<Node[]>(() => {
    const result: Node[] = []

    // Lifelines first so they render behind message arrows
    result.push({
      id: 'lifelines',
      type: 'lifelines',
      position: { x: 0, y: 0 },
      data: { columns, height: totalHeight, width: totalWidth },
      draggable: false,
      selectable: false,
      style: { width: totalWidth, height: totalHeight },
    })

    messages.forEach((msg, i) => {
      const xs = getArrowXs(msg, columns)
      if (!xs) return
      result.push({
        id: `msg-${i}`,
        type: 'message',
        position: { x: 0, y: TOP_MARGIN + i * ROW_HEIGHT - NODE_HEIGHT / 2 + ARROW_Y_OFFSET },
        data: {
          fromX: xs.fromX,
          toX: xs.toX,
          label: getMessageLabel(msg),
          color: getMessageColor(msg.method_or_status),
          selected: selectedIdx === i,
          timestampLabel: formatRelTime(msg.timestamp),
          width: totalWidth,
        },
        draggable: false,
        selectable: false,
        style: { width: totalWidth, height: NODE_HEIGHT },
      })
    })

    return result
  }, [messages, columns, selectedIdx, totalHeight, totalWidth, formatRelTime])

  // Auto-pan to latest message when live
  useEffect(() => {
    if (!live || messages.length === 0) return
    const lastY = TOP_MARGIN + (messages.length - 1) * ROW_HEIGHT
    const zoom = reactFlow.getZoom()
    reactFlow.setCenter(totalWidth / 2, lastY, { zoom, duration: 300 })
  }, [messages.length, live, totalWidth, reactFlow])

  return (
    <div className="flex gap-4">
      {/* Flow container */}
      <div
        className="flex-1 relative border rounded-lg overflow-hidden bg-card"
        style={{ height: '70vh' }}
      >
        <ReactFlow
          nodes={nodes}
          edges={[]}
          nodeTypes={nodeTypes}
          nodesDraggable={false}
          nodesConnectable={false}
          elementsSelectable={false}
          onNodeClick={(_, node) => {
            if (node.type !== 'message') return
            const idx = parseInt(node.id.replace('msg-', ''), 10)
            if (!isNaN(idx)) {
              setSelectedIdx((prev) => (prev === idx ? null : idx))
            }
          }}
          panOnDrag
          panOnScroll={false}
          zoomOnScroll
          zoomOnPinch
          zoomOnDoubleClick={false}
          minZoom={0.2}
          maxZoom={2}
          fitView
          fitViewOptions={{ padding: 0.15, maxZoom: 1 }}
        >
          <Background gap={20} />
          <Controls showInteractive={false} />
          <MiniMap pannable zoomable style={{ height: 80 }} />
        </ReactFlow>
        <StickyHeaders columns={columns} />
      </div>

      {/* Raw message detail panel */}
      {selectedIdx !== null && messages[selectedIdx] && (
        <div
          className="w-96 border rounded-lg bg-card flex flex-col"
          style={{ maxHeight: '70vh' }}
        >
          <div className="p-3 border-b flex items-center justify-between">
            <div>
              <p className="font-semibold text-sm">{messages[selectedIdx].method_or_status}</p>
              <p className="text-xs text-muted-foreground">
                {messages[selectedIdx].direction} | {messages[selectedIdx].transport.toUpperCase()} |{' '}
                {formatRelTime(messages[selectedIdx].timestamp)}
              </p>
            </div>
            <button
              className="text-muted-foreground hover:text-foreground text-lg px-2"
              onClick={() => setSelectedIdx(null)}
            >
              x
            </button>
          </div>
          <div className="p-3 text-xs space-y-2">
            <div className="grid grid-cols-2 gap-2">
              <div>
                <span className="text-muted-foreground">From:</span>
                <p className="font-mono break-all">{messages[selectedIdx].from_uri}</p>
              </div>
              <div>
                <span className="text-muted-foreground">To:</span>
                <p className="font-mono break-all">{messages[selectedIdx].to_uri}</p>
              </div>
              <div>
                <span className="text-muted-foreground">Source:</span>
                <p className="font-mono">{messages[selectedIdx].source_addr}</p>
              </div>
              <div>
                <span className="text-muted-foreground">Dest:</span>
                <p className="font-mono">{messages[selectedIdx].dest_addr}</p>
              </div>
              <div>
                <span className="text-muted-foreground">CSeq:</span>
                <p className="font-mono">{messages[selectedIdx].cseq}</p>
              </div>
              <div>
                <span className="text-muted-foreground">Time:</span>
                <p className="font-mono">{new Date(messages[selectedIdx].timestamp).toISOString()}</p>
              </div>
            </div>
          </div>
          <div className="flex-1 overflow-auto border-t">
            <pre className="p-3 text-xs font-mono whitespace-pre-wrap break-all text-foreground">
              {messages[selectedIdx].raw_message || '(no raw message captured)'}
            </pre>
          </div>
        </div>
      )}
    </div>
  )
}

export function SipLadder(props: SipLadderProps) {
  return (
    <ReactFlowProvider>
      <SipLadderInner {...props} />
    </ReactFlowProvider>
  )
}
