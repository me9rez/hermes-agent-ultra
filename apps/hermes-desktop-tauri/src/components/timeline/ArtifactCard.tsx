interface ArtifactCardProps {
  name: string
  previewUrl?: string
  onDownload?: () => void
  onPreview?: () => void
}

export function ArtifactCard({ name, previewUrl, onDownload, onPreview }: ArtifactCardProps) {
  return (
    <div className="terra-artifact-card">
      {previewUrl && <img src={previewUrl} alt="" />}
      <span>{name}</span>
      <button type="button" onClick={onPreview}>Preview</button>
      <button type="button" onClick={onDownload}>Download</button>
    </div>
  )
}

export default ArtifactCard
