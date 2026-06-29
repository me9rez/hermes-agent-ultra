import { createContext, useContext } from 'react'
export type UiMode = 'consumer' | 'dev'
export const UiModeContext = createContext<UiMode>('consumer')
export function useUiMode() { return useContext(UiModeContext) }
