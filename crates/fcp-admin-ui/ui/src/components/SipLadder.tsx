import { useState, useMemo, useRef, useEffect } from 'react'
import type { SipTraceMessage } from '@/api/endpoints'

interface SipLadderProps {
  messages: SipTraceMessage[]
  /** If true, auto-scroll to bottom when new messages arrive */
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

function getMessageLabel(msg: SipTraceMessage): string {
  const code = parseInt(msg.method_or_status, 10)
  if (!isNaN(code)) {
    // Extract method from CSeq for response labeling (e.g., "200 OK (INVITE)")
    const cseqMethod = msg.cseq.split(/\s+/)[1] || ''
    return `${msg.method_or_status} (${cseqMethod})`
  }
  return msg.method_or_status
}

/**
 * Determine the three columns for the ladder diagram.
 * Left = A-leg peer, Center = FCP, Right = B-leg peer.
 * We infer these from the addresses seen in the messages.
 */
function inferColumns(messages: SipTraceMessage[]): { left: string; center: string; right: string } {
  // Collect all unique addresses
  const localAddrs = new Set<string>()
  const peerAddrs = new Set<string>()

  for (const msg of messages) {
    if (msg.direction === 'inbound') {
      peerAddrs.add(msg.source_addr)
      localAddrs.add(msg.dest_addr)
    } else {
      localAddrs.add(msg.source_addr)
      peerAddrs.add(msg.dest_addr)
    }
  }

  // Pick a local address for center (our FCP node)
  const center = Array.from(localAddrs)[0] || 'FCP'

  // For peers, the first one seen on inbound is the A-leg (caller),
  // the other is the B-leg (callee)
  const peers = Array.from(peerAddrs)
  let left = peers[0] || 'Caller'
  let right = peers[1] || 'Callee'

  // If there's only one peer, it's a single-leg trace
  if (peers.length <= 1) {
    right = ''
  }

  return { left, center, right }
}

type ArrowDir = 'left-to-center' | 'center-to-left' | 'center-to-right' | 'right-to-center'

function getArrowDirection(msg: SipTraceMessage, columns: { left: string; center: string; right: string }): ArrowDir {
  if (msg.direction === 'inbound') {
    if (msg.source_addr === columns.left) return 'left-to-center'
    if (msg.source_addr === columns.right) return 'right-to-center'
    return 'left-to-center' // fallback
  } else {
    if (msg.dest_addr === columns.left) return 'center-to-left'
    if (msg.dest_addr === columns.right) return 'center-to-right'
    // If not matched, guess based on whether it's the first peer
    return 'center-to-right' // fallback
  }
}

const COL_LEFT_X = 120
const COL_CENTER_X = 350
const COL_RIGHT_X = 580
const ROW_HEIGHT = 40
const TOP_MARGIN = 50
const ARROW_HEAD_SIZE = 8

function Arrow({ fromX, toX, y, color, label, onClick, selected }: {
  fromX: number
  toX: number
  y: number
  color: string
  label: string
  onClick: () => void
  selected: boolean
}) {
  const midX = (fromX + toX) / 2
  const direction = toX > fromX ? 1 : -1
  const arrowTipX = toX
  const arrowBaseX = toX - direction * ARROW_HEAD_SIZE

  return (
    <g
      onClick={onClick}
      className="cursor-pointer"
      style={{ opacity: selected ? 1 : 0.85 }}
    >
      {/* Hover target (wider invisible line) */}
      <line
        x1={fromX} y1={y} x2={toX} y2={y}
        stroke="transparent" strokeWidth={16}
      />
      {/* Visible line */}
      <line
        x1={fromX} y1={y} x2={toX} y2={y}
        stroke={color}
        strokeWidth={selected ? 2.5 : 1.5}
      />
      {/* Arrow head */}
      <polygon
        points={`${arrowTipX},${y} ${arrowBaseX},${y - ARROW_HEAD_SIZE / 2} ${arrowBaseX},${y + ARROW_HEAD_SIZE / 2}`}
        fill={color}
      />
      {/* Label */}
      <text
        x={midX} y={y - 6}
        textAnchor="middle"
        fontSize={11}
        fontFamily="monospace"
        fill={color}
        fontWeight={selected ? 700 : 400}
      >
        {label}
      </text>
    </g>
  )
}

export function SipLadder({ messages, live }: SipLadderProps) {
  const [selectedIdx, setSelectedIdx] = useState<number | null>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const svgRef = useRef<SVGSVGElement>(null)

  const columns = useMemo(() => inferColumns(messages), [messages])
  const hasRight = columns.right !== ''

  // Auto-scroll when live and new messages arrive
  useEffect(() => {
    if (live && containerRef.current) {
      containerRef.current.scrollTop = containerRef.current.scrollHeight
    }
  }, [messages.length, live])

  const svgHeight = TOP_MARGIN + messages.length * ROW_HEIGHT + 30
  const svgWidth = hasRight ? 700 : 480

  // Format the timestamp relative to the first message
  const baseTime = messages.length > 0 ? messages[0].timestamp : 0
  const formatRelTime = (ts: number) => {
    const delta = ts - baseTime
    if (delta < 1000) return `+${delta}ms`
    return `+${(delta / 1000).toFixed(2)}s`
  }

  return (
    <div className="flex gap-4">
      {/* Ladder SVG */}
      <div ref={containerRef} className="flex-1 overflow-auto border rounded-lg bg-card" style={{ maxHeight: '70vh' }}>
        <svg ref={svgRef} width={svgWidth} height={svgHeight} className="select-none">
          {/* Column headers */}
          <text x={COL_LEFT_X} y={20} textAnchor="middle" fontSize={12} fontWeight={700} fill="currentColor" className="fill-foreground">
            {columns.left.length > 21 ? columns.left.slice(0, 21) + '...' : columns.left}
          </text>
          <text x={COL_CENTER_X} y={20} textAnchor="middle" fontSize={12} fontWeight={700} fill="currentColor" className="fill-foreground">
            {columns.center.length > 21 ? columns.center.slice(0, 21) + '...' : columns.center}
          </text>
          {hasRight && (
            <text x={COL_RIGHT_X} y={20} textAnchor="middle" fontSize={12} fontWeight={700} fill="currentColor" className="fill-foreground">
              {columns.right.length > 21 ? columns.right.slice(0, 21) + '...' : columns.right}
            </text>
          )}

          {/* Vertical lifelines */}
          <line x1={COL_LEFT_X} y1={30} x2={COL_LEFT_X} y2={svgHeight - 10} stroke="currentColor" strokeWidth={1} strokeDasharray="4 4" opacity={0.3} className="stroke-foreground" />
          <line x1={COL_CENTER_X} y1={30} x2={COL_CENTER_X} y2={svgHeight - 10} stroke="currentColor" strokeWidth={1} strokeDasharray="4 4" opacity={0.3} className="stroke-foreground" />
          {hasRight && (
            <line x1={COL_RIGHT_X} y1={30} x2={COL_RIGHT_X} y2={svgHeight - 10} stroke="currentColor" strokeWidth={1} strokeDasharray="4 4" opacity={0.3} className="stroke-foreground" />
          )}

          {/* Messages as arrows */}
          {messages.map((msg, i) => {
            const y = TOP_MARGIN + i * ROW_HEIGHT
            const dir = getArrowDirection(msg, columns)
            const color = getMessageColor(msg.method_or_status)
            const label = getMessageLabel(msg)

            let fromX: number, toX: number
            switch (dir) {
              case 'left-to-center': fromX = COL_LEFT_X; toX = COL_CENTER_X; break
              case 'center-to-left': fromX = COL_CENTER_X; toX = COL_LEFT_X; break
              case 'center-to-right': fromX = COL_CENTER_X; toX = COL_RIGHT_X; break
              case 'right-to-center': fromX = COL_RIGHT_X; toX = COL_CENTER_X; break
            }

            return (
              <g key={msg.id || i}>
                {/* Selected row highlight */}
                {selectedIdx === i && (
                  <rect
                    x={0} y={y - ROW_HEIGHT / 2 + 2} width={svgWidth} height={ROW_HEIGHT - 4}
                    fill="currentColor" opacity={0.05} className="fill-foreground"
                  />
                )}
                {/* Timestamp */}
                <text
                  x={8} y={y + 4}
                  fontSize={9} fontFamily="monospace"
                  fill="currentColor" opacity={0.5} className="fill-foreground"
                >
                  {formatRelTime(msg.timestamp)}
                </text>
                <Arrow
                  fromX={fromX}
                  toX={toX}
                  y={y}
                  color={color}
                  label={label}
                  onClick={() => setSelectedIdx(selectedIdx === i ? null : i)}
                  selected={selectedIdx === i}
                />
              </g>
            )
          })}
        </svg>
      </div>

      {/* Raw message detail panel */}
      {selectedIdx !== null && messages[selectedIdx] && (
        <div className="w-96 border rounded-lg bg-card flex flex-col" style={{ maxHeight: '70vh' }}>
          <div className="p-3 border-b flex items-center justify-between">
            <div>
              <p className="font-semibold text-sm">
                {messages[selectedIdx].method_or_status}
              </p>
              <p className="text-xs text-muted-foreground">
                {messages[selectedIdx].direction} | {messages[selectedIdx].transport.toUpperCase()} | {formatRelTime(messages[selectedIdx].timestamp)}
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
