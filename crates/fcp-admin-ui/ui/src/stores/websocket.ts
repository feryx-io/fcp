import { create } from 'zustand'
import type { PlatformEvent, ToastMessage } from '@/types'
import { apiClient } from '@/api/client'

interface WebSocketState {
  connected: boolean
  events: PlatformEvent[]
  toasts: ToastMessage[]
  connect: () => void
  disconnect: () => void
  dismissToast: (id: string) => void
}

let ws: WebSocket | null = null
let reconnectTimer: ReturnType<typeof setTimeout> | null = null

const MAX_EVENTS = 200
const MAX_TOASTS = 5
const RECONNECT_DELAY = 3000

function getWsUrl(): string {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${protocol}//${window.location.host}/api/v1/events`
}

function createToastFromEvent(event: PlatformEvent): ToastMessage | null {
  const id = crypto.randomUUID()
  const timestamp = Date.now()

  switch (event.event_type) {
    case 'call_started':
      return {
        id, timestamp,
        title: 'New Call',
        description: `Call ${(event as Record<string, unknown>).call_id ?? ''} started`,
        variant: 'default',
      }
    case 'call_ended':
      return {
        id, timestamp,
        title: 'Call Ended',
        description: `Call ${(event as Record<string, unknown>).call_id ?? ''} ended`,
        variant: 'default',
      }
    case 'call_failed':
      return {
        id, timestamp,
        title: 'Call Failed',
        description: `Call ${(event as Record<string, unknown>).call_id ?? ''} failed: ${(event as Record<string, unknown>).reason ?? 'unknown'}`,
        variant: 'destructive',
      }
    case 'agent_logged_in':
      return {
        id, timestamp,
        title: 'Agent Online',
        description: `Agent ${(event as Record<string, unknown>).agent_id ?? ''} logged in`,
        variant: 'success',
      }
    case 'queue_threshold':
      return {
        id, timestamp,
        title: 'Queue Alert',
        description: `Queue ${(event as Record<string, unknown>).queue_name ?? ''} threshold exceeded`,
        variant: 'warning',
      }
    default:
      return null
  }
}

export const useWebSocketStore = create<WebSocketState>()((set, get) => ({
  connected: false,
  events: [],
  toasts: [],

  connect: () => {
    if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) {
      return
    }

    try {
      ws = new WebSocket(getWsUrl())

      ws.onopen = () => {
        // Send auth token as the first message (not in URL, to avoid
        // leaking credentials via access logs, proxies, and monitoring).
        const token = apiClient.getToken()
        if (token) {
          ws?.send(JSON.stringify({ type: 'auth', token }))
        }
        if (reconnectTimer) {
          clearTimeout(reconnectTimer)
          reconnectTimer = null
        }
      }

      ws.onmessage = (msg) => {
        try {
          const data = JSON.parse(msg.data) as Record<string, unknown>

          // Handle auth response from server
          if (data.type === 'auth_error') {
            ws?.close()
            return
          }
          if (data.type === 'auth_ok') {
            // Auth succeeded, wait for 'connected' message
            return
          }
          // Mark as connected once the server confirms subscription
          if (data.type === 'connected') {
            set({ connected: true })
            return
          }

          // Process platform events
          const event = data as unknown as PlatformEvent
          if (!event.event_type) return
          const { events } = get()
          const newEvents = [event, ...events].slice(0, MAX_EVENTS)
          set({ events: newEvents })

          // Create toast for notable events
          const toast = createToastFromEvent(event)
          if (toast) {
            const { toasts } = get()
            const newToasts = [toast, ...toasts].slice(0, MAX_TOASTS)
            set({ toasts: newToasts })

            // Auto-dismiss after 5 seconds
            setTimeout(() => {
              get().dismissToast(toast.id)
            }, 5000)
          }
        } catch {
          // Ignore malformed messages
        }
      }

      ws.onclose = () => {
        set({ connected: false })
        ws = null
        // Auto-reconnect
        reconnectTimer = setTimeout(() => {
          get().connect()
        }, RECONNECT_DELAY)
      }

      ws.onerror = () => {
        ws?.close()
      }
    } catch {
      set({ connected: false })
    }
  },

  disconnect: () => {
    if (reconnectTimer) {
      clearTimeout(reconnectTimer)
      reconnectTimer = null
    }
    if (ws) {
      ws.close()
      ws = null
    }
    set({ connected: false })
  },

  dismissToast: (id: string) => {
    set({ toasts: get().toasts.filter((t) => t.id !== id) })
  },
}))
