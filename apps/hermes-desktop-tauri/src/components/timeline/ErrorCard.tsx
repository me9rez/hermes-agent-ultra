interface ErrorCardProps {
  message: string
  details?: string
}

export function ErrorCard({ message, details }: ErrorCardProps) {
  return (
    <details className="terra-error-card">
      <summary>{message}</summary>
      {details && <pre>{details}</pre>}
    </details>
  )
}

export default ErrorCard
