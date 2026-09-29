/** One page of a list (docs/API_TOKENS.md §6). */
export interface PageOf<T> {
  data: T[]
  next_cursor?: string | null
  total?: number | null
}

/**
 * Every item of a list, fetching page after page by `next_cursor`:
 *
 * ```ts
 * for await (const user of paginate((cursor) =>
 *   ok(fcp.GET('/api/v1/users', { params: { query: { cursor, limit: 500 } } }))))
 * ```
 */
export async function* paginate<T>(
  page: (cursor: string | undefined) => Promise<PageOf<T>>,
): AsyncGenerator<T> {
  let cursor: string | undefined
  do {
    const current = await page(cursor)
    yield* current.data
    cursor = current.next_cursor ?? undefined
  } while (cursor)
}

/** Every item of a list, gathered. */
export async function all<T>(page: (cursor: string | undefined) => Promise<PageOf<T>>): Promise<T[]> {
  const out: T[] = []
  for await (const item of paginate(page)) out.push(item)
  return out
}
