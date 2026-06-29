import { useState } from 'react'

import { useT } from '@/i18n/useT'

export function OllamaSettings() {
  const t = useT('settings')
  const [host, setHost] = useState('127.0.0.1:11434')
  const [status, setStatus] = useState<string | null>(null)

  const test = async () => {
    try {
      const res = await fetch(`http://${host}/api/tags`)
      setStatus(res.ok ? t('ollama.ok', 'Connected') : t('ollama.fail', 'Unreachable'))
    } catch {
      setStatus(t('ollama.fail', 'Unreachable'))
    }
  }

  return (
    <section className="terra-ollama-settings">
      <h3>{t('ollama.title', 'Ollama (local)')}</h3>
      <input value={host} onChange={e => setHost(e.target.value)} aria-label={t('ollama.host', 'Host')} />
      <button type="button" onClick={() => void test()}>
        {t('ollama.test', 'Test connection')}
      </button>
      {status ? <p>{status}</p> : null}
    </section>
  )
}

export default OllamaSettings
