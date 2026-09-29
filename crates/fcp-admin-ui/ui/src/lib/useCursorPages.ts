import { useCallback, useState } from 'react'

/**
 * Previous and next over a cursor-paged list (docs/DEVELOPER_SURFACE.md
 * §3, phase 2c): the cursors of the pages before this one are kept, so
 * "previous" goes back to exactly the page it came from.
 */
export function useCursorPages() {
  const [before, setBefore] = useState<(string | undefined)[]>([])
  const [cursor, setCursor] = useState<string | undefined>(undefined)

  const next = useCallback(
    (nextCursor: string) => {
      setBefore((b) => [...b, cursor])
      setCursor(nextCursor)
    },
    [cursor],
  )
  const previous = useCallback(() => {
    setBefore((b) => {
      const kept = b.slice(0, -1)
      setCursor(b[b.length - 1])
      return kept
    })
  }, [])
  const reset = useCallback(() => {
    setBefore([])
    setCursor(undefined)
  }, [])

  return { cursor, next, previous, reset, pageIndex: before.length }
}
