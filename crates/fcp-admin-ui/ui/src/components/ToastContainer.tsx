import { X } from 'lucide-react'
import { cn } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import { useWebSocketStore } from '@/stores/websocket'

const variantStyles = {
  default: 'border-border bg-card',
  success: 'border-green-500/50 bg-green-50 dark:bg-green-950/30',
  warning: 'border-yellow-500/50 bg-yellow-50 dark:bg-yellow-950/30',
  destructive: 'border-destructive/50 bg-red-50 dark:bg-red-950/30',
}

export function ToastContainer() {
  const { toasts, dismissToast } = useWebSocketStore()

  if (toasts.length === 0) return null

  return (
    <div className="fixed bottom-4 right-4 z-50 flex flex-col gap-2 w-80" role="status" aria-live="polite">
      {toasts.map((toast) => (
        <div
          key={toast.id}
          className={cn(
            'rounded-lg border p-4 shadow-lg animate-in slide-in-from-right-full',
            variantStyles[toast.variant],
          )}
        >
          <div className="flex items-start justify-between gap-2">
            <div className="flex-1">
              <p className="text-sm font-semibold">{toast.title}</p>
              <p className="text-xs text-muted-foreground mt-1">{toast.description}</p>
            </div>
            <Button
              variant="ghost"
              size="icon"
              className="h-6 w-6 shrink-0"
              onClick={() => dismissToast(toast.id)}
            >
              <X className="h-3 w-3" />
            </Button>
          </div>
        </div>
      ))}
    </div>
  )
}
