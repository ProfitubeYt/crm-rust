import React, { useEffect } from 'react'
import ReactDOM from 'react-dom/client'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { CssBaseline, ThemeProvider as MuiThemeProvider } from '@mui/material'
import { ThemeProvider as AppThemeProvider, useTheme } from './contexts/ThemeContext'
import { ToastProvider, useToast } from './components/Toast'
import { UserbotPage } from './pages/UserbotPage'
import { postToShell } from './services/nativeShell'
import { muiTheme } from './styles/muiTheme'
import './index.css'

/** The Telegram section alone, for the native client's embedded web view. */

const queryClient = new QueryClient({
    defaultOptions: {
        queries: {
            refetchOnWindowFocus: false,
            retry: 1,
            staleTime: 30000,
        },
    },
})

const requestedTheme = new URLSearchParams(window.location.search).get('theme')
if (requestedTheme === 'light' || requestedTheme === 'dark') {
    localStorage.setItem('app-theme', requestedTheme)
}

const ShellBridge: React.FC = () => {
    const { setTheme } = useTheme()
    const { showToast } = useToast()

    useEffect(() => {
        window.__tbsShell = { theme: setTheme, toast: showToast }
        postToShell({ type: 'ready' })
        return () => {
            delete window.__tbsShell
        }
        // Both are recreated on every provider render; the first pair stays valid.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [])

    return null
}

ReactDOM.createRoot(document.getElementById('root')!).render(
    <React.StrictMode>
        <AppThemeProvider>
            <MuiThemeProvider theme={muiTheme}>
                <CssBaseline />
                <QueryClientProvider client={queryClient}>
                    <ToastProvider>
                        <ShellBridge />
                        <UserbotPage />
                    </ToastProvider>
                </QueryClientProvider>
            </MuiThemeProvider>
        </AppThemeProvider>
    </React.StrictMode>,
)
