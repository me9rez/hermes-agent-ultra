interface SubagentSpawnCardProps {
  role: string
}

export function SubagentSpawnCard({ role }: SubagentSpawnCardProps) {
  return <div className="terra-subagent-spawn-card">{role}</div>
}

export default SubagentSpawnCard
