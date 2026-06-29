const PREWARM_DELAY_MS = 5000

export interface PrewarmTargets {
  fetchAccount?: () => Promise<void>
  fetchActiveTasks?: () => Promise<void>
  fetchProviderPrefs?: () => Promise<void>
}

export function scheduleBackgroundPrewarm(targets: PrewarmTargets): () => void {
  let cancelled = false
  const timer = window.setTimeout(async () => {
    if (cancelled) return
    await Promise.allSettled([
      targets.fetchAccount?.(),
      targets.fetchActiveTasks?.(),
      targets.fetchProviderPrefs?.()
    ])
  }, PREWARM_DELAY_MS)
  return () => {
    cancelled = true
    window.clearTimeout(timer)
  }
}
