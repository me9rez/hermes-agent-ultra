const CACHE_KEY = 'terra.cache.bootstrap.v1'

export interface CachedBootstrap {
  activeTaskId?: string
  accountAvatarUrl?: string
  accountName?: string
  cachedAt: number
}

export function readCachedBootstrap(): CachedBootstrap | null {
  try {
    const raw = localStorage.getItem(CACHE_KEY)
    if (!raw) return null
    return JSON.parse(raw) as CachedBootstrap
  } catch {
    return null
  }
}

export function writeCachedBootstrap(data: Partial<CachedBootstrap>): void {
  const prev = readCachedBootstrap() ?? { cachedAt: Date.now() }
  const next: CachedBootstrap = {
    ...prev,
    ...data,
    cachedAt: Date.now()
  }
  localStorage.setItem(CACHE_KEY, JSON.stringify(next))
}

export function hasColdCache(): boolean {
  return readCachedBootstrap() !== null
}

export function clearCachedBootstrap(): void {
  localStorage.removeItem(CACHE_KEY)
}
