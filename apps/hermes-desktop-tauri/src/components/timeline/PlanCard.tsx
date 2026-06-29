interface PlanCardProps {
  steps: string[]
}

export function PlanCard({ steps }: PlanCardProps) {
  return (
    <ol className="terra-plan-card">
      {steps.map((step, i) => (
        <li key={i}>{step}</li>
      ))}
    </ol>
  )
}

export default PlanCard
