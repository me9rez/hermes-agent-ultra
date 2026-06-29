import fs from 'node:fs'
import path from 'node:path'

const variantId = process.argv[2] ?? 'default'
const root = path.resolve(import.meta.dirname, '..')
const variantPath = path.join(root, 'variants', variantId, 'variant.json')
const variant = JSON.parse(fs.readFileSync(variantPath, 'utf8'))
const outDir = path.join(root, 'src', '.generated')
fs.mkdirSync(outDir, { recursive: true })
fs.writeFileSync(
  path.join(outDir, 'branding.ts'),
  `export const VARIANT = ${JSON.stringify(variant, null, 2)} as const\n`
)
console.log(`Applied variant ${variantId}`)
