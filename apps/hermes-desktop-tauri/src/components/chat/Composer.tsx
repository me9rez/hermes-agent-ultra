import { useCallback, useEffect, useState, type KeyboardEvent } from 'react'

import { useT } from '@/i18n/useT'

interface ComposerProps {
  value?: string
  onChange?: (value: string) => void
  onSubmit?: () => void
  onStop?: () => void
  running?: boolean
}

export function Composer({ value = '', onChange, onSubmit, onStop, running }: ComposerProps) {
  const t = useT('composer')
  const [draft, setDraft] = useState(value)

  useEffect(() => {
    setDraft(value)
  }, [value])

  const commit = useCallback(() => {
    onChange?.(draft)
    onSubmit?.()
  }, [draft, onChange, onSubmit])

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
      event.preventDefault()
      commit()
    }
  }

  return (
    <div className="terra-composer">
      <textarea
        value={draft}
        rows={3}
        onChange={event => {
          setDraft(event.target.value)
          onChange?.(event.target.value)
        }}
        onKeyDown={onKeyDown}
      />
      {running ? (
        <button type="button" onClick={onStop}>
          {t('stop')}
        </button>
      ) : (
        <button type="button" onClick={commit}>
          {t('send')}
        </button>
      )}
    </div>
  )
}

export default Composer
