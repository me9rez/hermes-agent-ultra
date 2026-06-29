interface ToolResultBlockProps {
  result: string
}

export function ToolResultBlock({ result }: ToolResultBlockProps) {
  return <pre className="terra-tool-result-block">{result}</pre>
}

export default ToolResultBlock
