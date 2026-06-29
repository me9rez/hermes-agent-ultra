interface ApprovalRequestCardProps {
  summary: string
  onApprove?: () => void
  onReject?: () => void
}

export function ApprovalRequestCard({ summary, onApprove, onReject }: ApprovalRequestCardProps) {
  return (
    <div className="terra-approval-request-card">
      <p>{summary}</p>
      <button type="button" onClick={onApprove}>Approve</button>
      <button type="button" onClick={onReject}>Reject</button>
    </div>
  )
}

export default ApprovalRequestCard
