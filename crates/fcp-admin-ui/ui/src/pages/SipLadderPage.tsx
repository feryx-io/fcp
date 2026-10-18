import { useState, useEffect, useCallback } from 'react'
import { useParams, useNavigate } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { ArrowLeft, Copy, Download, Radio } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { SipLadder } from '@/components/SipLadder'
import { sipTrace } from '@/api/endpoints'
import { apiClient } from '@/api/client'
import type { SipTraceMessage } from '@/api/endpoints'

export function SipLadderPage() {
  const { callId } = useParams<{ callId: string }>()
  const navigate = useNavigate()
  const [liveMessages, setLiveMessages] = useState<SipTraceMessage[]>([])
  const [isLive, setIsLive] = useState(false)

  const { data, isLoading, error } = useQuery({
    queryKey: ['sip-trace', callId],
    queryFn: () => sipTrace.getMessages(callId!),
    enabled: !!callId,
  })

  // Live WebSocket subscription for active calls
  useEffect(() => {
    if (!isLive || !callId) return

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const wsUrl = `${protocol}//${window.location.host}/api/v1/events?event_types=sip`
    const ws = new WebSocket(wsUrl)

    ws.onopen = () => {
      // Send auth token as first message (not in URL, to avoid log leakage)
      const token = apiClient.getToken()
      if (token) {
        ws.send(JSON.stringify({ type: 'auth', token }))
      }
    }

    ws.onmessage = (evt) => {
      try {
        const event = JSON.parse(evt.data)
        // Skip auth/control messages
        if (event.type === 'auth_ok' || event.type === 'auth_error' || event.type === 'connected') return
        if (event.event_type === 'sip.message_captured' && event.call_id === callId) {
          const msg: SipTraceMessage = {
            id: Date.now(), // temporary ID for live messages
            timestamp: event.timestamp ?? Date.now(),
            direction: event.direction,
            method_or_status: event.method_or_status,
            cseq: event.cseq ?? '',
            from_uri: event.from_uri ?? '',
            to_uri: event.to_uri ?? '',
            source_addr: event.source_addr ?? '',
            dest_addr: event.dest_addr ?? '',
            transport: event.transport ?? '',
            raw_message: event.raw_message ?? '',
            node_id: event.node_id ?? null,
          }
          setLiveMessages((prev) => [...prev, msg])
        }
      } catch {
        // ignore parse errors
      }
    }

    ws.onclose = () => setIsLive(false)
    ws.onerror = () => ws.close()

    return () => {
      ws.close()
    }
  }, [isLive, callId])

  // Combine REST data + live WebSocket data
  const allMessages = [
    ...(data?.messages ?? []),
    ...liveMessages,
  ]

  const copyAllMessages = useCallback(() => {
    const text = allMessages
      .map((m) => `[${new Date(m.timestamp).toISOString()}] ${m.direction} ${m.method_or_status}\n${m.raw_message}`)
      .join('\n\n---\n\n')
    navigator.clipboard.writeText(text)
  }, [allMessages])

  const exportAsText = useCallback(() => {
    const text = allMessages
      .map((m) => `[${new Date(m.timestamp).toISOString()}] ${m.direction} ${m.method_or_status} (${m.source_addr} -> ${m.dest_addr})\n${m.raw_message}`)
      .join('\n\n' + '='.repeat(80) + '\n\n')

    const blob = new Blob([text], { type: 'text/plain' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `sip-trace-${callId}.txt`
    a.click()
    URL.revokeObjectURL(url)
  }, [allMessages, callId])

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Button variant="ghost" size="icon" title="Back" onClick={() => navigate(-1)}>
            <ArrowLeft className="h-4 w-4" />
          </Button>
          <div>
            <h1 className="text-2xl font-bold tracking-tight">SIP Trace</h1>
            <p className="text-sm text-muted-foreground font-mono">{callId}</p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant={isLive ? 'default' : 'outline'}
            size="sm"
            onClick={() => {
              setIsLive(!isLive)
              if (!isLive) setLiveMessages([])
            }}
          >
            <Radio className="mr-1 h-3 w-3" />
            {isLive ? 'Live' : 'Go Live'}
          </Button>
          <Button variant="outline" size="sm" onClick={copyAllMessages} disabled={allMessages.length === 0}>
            <Copy className="mr-1 h-3 w-3" />
            Copy
          </Button>
          <Button variant="outline" size="sm" onClick={exportAsText} disabled={allMessages.length === 0}>
            <Download className="mr-1 h-3 w-3" />
            Export
          </Button>
        </div>
      </div>

      {/* Status */}
      <div className="flex items-center gap-2">
        <Badge variant="outline">{allMessages.length} messages</Badge>
        {isLive && (
          <Badge variant="success" className="animate-pulse">
            <span className="mr-1 h-2 w-2 rounded-full bg-green-500 inline-block" />
            Streaming
          </Badge>
        )}
      </div>

      {/* Content */}
      {isLoading ? (
        <div className="flex h-64 items-center justify-center text-muted-foreground">
          <div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" />
        </div>
      ) : error ? (
        <div className="flex h-64 items-center justify-center text-destructive">
          Failed to load SIP traces. The SIP trace store may not be configured.
        </div>
      ) : allMessages.length === 0 ? (
        <div className="flex h-64 items-center justify-center text-muted-foreground">
          No SIP messages captured for this call.
        </div>
      ) : (
        <SipLadder messages={allMessages} live={isLive} />
      )}
    </div>
  )
}
