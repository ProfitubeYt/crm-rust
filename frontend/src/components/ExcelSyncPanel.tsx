import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react';
import axios from 'axios';
import { Check, Copy, Download, KeyRound, RefreshCw, Unplug } from 'lucide-react';
import { excelSyncApi, type ExcelSyncActivation, type ExcelSyncGrant, type ExcelSyncGrantCreated } from '../services/api';
import { Sheet } from './motion/Sheet';
import '../styles/settings.css';

interface ExcelSyncPanelProps {
    isOpen: boolean;
    onClose: () => void;
}

function requestError(error: unknown, fallback: string): string {
    if (!axios.isAxiosError(error)) return fallback;
    if (error.response?.status === 401) return 'Войдите в приложение заново.';
    if (error.response?.status === 403) return 'Доступ к подключению Excel закрыт. Проверьте доступ к транзакциям.';
    if (error.response?.status === 429) return 'Слишком много запросов. Попробуйте немного позже.';
    if (!error.response) return 'Нет связи с сервером. Проверьте подключение и повторите попытку.';
    return fallback;
}

function formatDate(value: string | null): string {
    if (!value) return 'Ещё не подключалась';
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? '—' : date.toLocaleString('ru-RU', { dateStyle: 'short', timeStyle: 'short' });
}

interface ConnectedWorkbook {
    id: string;
    name: string;
}

/** One entry per connected workbook; computers activated later share its ID. */
function connectedWorkbooks(grants: ExcelSyncGrant[]): ConnectedWorkbook[] {
    const books = new Map<string, ConnectedWorkbook & { fromDevice: boolean }>();
    for (const grant of grants) {
        if (!grant.workbook_id) continue;
        const known = books.get(grant.workbook_id);
        const fromDevice = Boolean(grant.device_bound);
        if (!known || (known.fromDevice && !fromDevice)) {
            books.set(grant.workbook_id, { id: grant.workbook_id, name: grant.name, fromDevice });
        }
    }
    return [...books.values()].map(({ id, name }) => ({ id, name }));
}

const MIN_PASSWORD = 8;

export function ExcelSyncPanel({ isOpen, onClose }: ExcelSyncPanelProps) {
    const [grants, setGrants] = useState<ExcelSyncGrant[]>([]);
    const [loading, setLoading] = useState(false);
    const [listError, setListError] = useState<string | null>(null);
    const [name, setName] = useState('Моя книга Excel');
    const [expiresInDays, setExpiresInDays] = useState(90);
    const [creating, setCreating] = useState(false);
    const [revokingId, setRevokingId] = useState<ExcelSyncGrant['id'] | null>(null);
    const [downloading, setDownloading] = useState(false);
    const [downloadError, setDownloadError] = useState<string | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [activations, setActivations] = useState<Record<string, ExcelSyncActivation>>({});
    const [activationsError, setActivationsError] = useState<string | null>(null);
    // Passwords live only in this component's state until they are sent.
    const [passwords, setPasswords] = useState<Record<string, string>>({});
    const [passwordBook, setPasswordBook] = useState<string | null>(null);
    const [passwordError, setPasswordError] = useState<{ book: string; message: string } | null>(null);
    const [passwordSaved, setPasswordSaved] = useState<string | null>(null);
    // Never put the one-time token in the query cache, a store, or browser storage.
    const [connection, setConnection] = useState<ExcelSyncGrantCreated | null>(null);
    const [issued, setIssued] = useState(false);
    const [copied, setCopied] = useState<'server' | 'token' | null>(null);
    const [copyError, setCopyError] = useState<string | null>(null);
    const sessionRef = useRef(0);
    const mutationRef = useRef(false);
    const issuedRef = useRef(false);
    const downloadRef = useRef(false);

    const loadGrants = useCallback(async (session: number, signal?: AbortSignal) => {
        setLoading(true);
        setListError(null);
        setActivationsError(null);
        const activationRows = excelSyncApi.listActivations(signal).then(
            (rows) => {
                if (session === sessionRef.current) setActivations(Object.fromEntries(rows.map((row) => [row.workbook_id, row])));
            },
            (err: unknown) => {
                if (session === sessionRef.current && !signal?.aborted) {
                    setActivationsError(requestError(err, 'Не удалось загрузить пароли книг.'));
                }
            },
        );
        try {
            const rows = await excelSyncApi.listGrants(signal);
            if (session === sessionRef.current) setGrants(rows);
            await activationRows;
        } catch (err) {
            if (session === sessionRef.current && !signal?.aborted) {
                setListError(requestError(err, 'Не удалось загрузить подключения.'));
            }
        } finally {
            if (session === sessionRef.current) setLoading(false);
        }
    }, []);

    useEffect(() => {
        const session = ++sessionRef.current;
        const controller = new AbortController();
        setConnection(null);
        setCopied(null);
        setCopyError(null);
        setIssued(false);
        setError(null);
        setDownloadError(null);
        setGrants([]);
        setActivations({});
        setActivationsError(null);
        setPasswords({});
        setPasswordBook(null);
        setPasswordError(null);
        setPasswordSaved(null);
        setCreating(false);
        setRevokingId(null);
        setDownloading(false);
        mutationRef.current = false;
        issuedRef.current = false;
        downloadRef.current = false;
        if (isOpen) {
            setName('Моя книга Excel');
            setExpiresInDays(90);
            void loadGrants(session, controller.signal);
        }
        return () => {
            sessionRef.current = session + 1;
            controller.abort();
        };
    }, [isOpen, loadGrants]);

    const close = () => {
        if (mutationRef.current) return;
        ++sessionRef.current;
        setConnection(null);
        setCopied(null);
        setCopyError(null);
        setPasswords({});
        onClose();
    };

    const createGrant = async (event: FormEvent) => {
        event.preventDefault();
        if (mutationRef.current || loading || issuedRef.current) return;
        const label = name.trim();
        if (!label || label.length > 120) {
            setError('Введите название книги: от 1 до 120 символов.');
            return;
        }
        if (!Number.isInteger(expiresInDays) || expiresInDays < 1 || expiresInDays > 365) {
            setError('Срок подключения должен быть от 1 до 365 дней.');
            return;
        }
        const session = sessionRef.current;
        mutationRef.current = true;
        setCreating(true);
        setError(null);
        try {
            const result = await excelSyncApi.createGrant(label, expiresInDays);
            if (session !== sessionRef.current) return;
            issuedRef.current = true;
            setGrants((current) => [result.grant, ...current.filter((row) => row.id !== result.grant.id)]);
            setConnection(result);
            setIssued(true);
        } catch (err) {
            if (session === sessionRef.current) {
                setError(requestError(err, 'Не удалось создать подключение. Обновите список перед повторной попыткой.'));
            }
        } finally {
            if (session === sessionRef.current) {
                mutationRef.current = false;
                setCreating(false);
            }
        }
    };

    const revokeGrant = async (grant: ExcelSyncGrant) => {
        if (mutationRef.current) return;
        const session = sessionRef.current;
        mutationRef.current = true;
        setRevokingId(grant.id);
        setConnection(null);
        setCopied(null);
        setCopyError(null);
        setError(null);
        try {
            await excelSyncApi.revokeGrant(grant.id);
            if (session !== sessionRef.current) return;
            setGrants((current) => current.map((row) => row.id === grant.id
                ? { ...row, revoked_at: new Date().toISOString() }
                : row));
        } catch (err) {
            if (session === sessionRef.current) setError(requestError(err, 'Не удалось отозвать подключение.'));
        } finally {
            if (session === sessionRef.current) {
                mutationRef.current = false;
                setRevokingId(null);
            }
        }
    };

    const saveActivationPassword = async (event: FormEvent, book: ConnectedWorkbook) => {
        event.preventDefault();
        if (mutationRef.current) return;
        const password = passwords[book.id] ?? '';
        if (password.trim().length < MIN_PASSWORD || password.length > 128) {
            setPasswordError({ book: book.id, message: `Пароль должен содержать от ${MIN_PASSWORD} до 128 символов.` });
            return;
        }
        const session = sessionRef.current;
        mutationRef.current = true;
        setPasswordBook(book.id);
        setPasswordError(null);
        setPasswordSaved(null);
        try {
            const row = await excelSyncApi.setActivationPassword(book.id, password);
            if (session !== sessionRef.current) return;
            setActivations((current) => ({ ...current, [row.workbook_id]: row }));
            setPasswords((current) => ({ ...current, [book.id]: '' }));
            setPasswordSaved(book.id);
        } catch (err) {
            if (session !== sessionRef.current) return;
            const status = axios.isAxiosError(err) ? err.response?.status : undefined;
            setPasswordError({
                book: book.id,
                message: status === 404 ? 'Книга не найдена. Обновите список подключений.'
                    : status === 422 ? `Пароль должен содержать от ${MIN_PASSWORD} до 128 символов.`
                        : requestError(err, 'Не удалось сохранить пароль.'),
            });
        } finally {
            if (session === sessionRef.current) {
                mutationRef.current = false;
                setPasswordBook(null);
            }
        }
    };

    const removeActivationPassword = async (book: ConnectedWorkbook) => {
        if (mutationRef.current) return;
        const session = sessionRef.current;
        mutationRef.current = true;
        setPasswordBook(book.id);
        setPasswordError(null);
        setPasswordSaved(null);
        try {
            await excelSyncApi.removeActivationPassword(book.id);
            if (session !== sessionRef.current) return;
            setActivations((current) => {
                const next = { ...current };
                delete next[book.id];
                return next;
            });
        } catch (err) {
            if (session === sessionRef.current) {
                setPasswordError({ book: book.id, message: requestError(err, 'Не удалось убрать пароль.') });
            }
        } finally {
            if (session === sessionRef.current) {
                mutationRef.current = false;
                setPasswordBook(null);
            }
        }
    };

    const downloadWorkbook = async () => {
        if (downloadRef.current) return;
        const session = sessionRef.current;
        downloadRef.current = true;
        setDownloading(true);
        setDownloadError(null);
        try {
            const blob = await excelSyncApi.downloadWorkbook();
            if (session !== sessionRef.current) return;
            const url = URL.createObjectURL(blob);
            const link = document.createElement('a');
            link.href = url;
            link.download = 'Чеки — синхронизация.xlsm';
            document.body.appendChild(link);
            link.click();
            link.remove();
            window.setTimeout(() => URL.revokeObjectURL(url), 1000);
        } catch (err) {
            if (session === sessionRef.current) {
                const unavailable = axios.isAxiosError(err) && [404, 503].includes(err.response?.status || 0);
                setDownloadError(unavailable
                    ? 'Книга пока недоступна для скачивания. Если вы уже получили файл «Чеки — синхронизация.xlsm», подключите его ниже.'
                    : requestError(err, 'Не удалось скачать книгу. Попробуйте ещё раз.'));
            }
        } finally {
            if (session === sessionRef.current) {
                downloadRef.current = false;
                setDownloading(false);
            }
        }
    };

    const copy = async (field: 'server' | 'token') => {
        if (!connection) return;
        const session = sessionRef.current;
        setCopyError(null);
        try {
            await navigator.clipboard.writeText(field === 'token' ? connection.token : connection.server_url);
            if (session === sessionRef.current) setCopied(field);
        } catch {
            if (session === sessionRef.current) setCopyError('Выделите значение в поле и скопируйте его сочетанием Ctrl+C.');
        }
    };

    const busy = creating || revokingId !== null || passwordBook !== null;
    const books = connectedWorkbooks(grants);

    return (
        <Sheet
            open={isOpen}
            onClose={close}
            title="Синхронизация с Excel"
            subtitle="Книга получает новые чеки и отправляет ваши изменения в приложение. Доступ ограничен вашими правами на транзакции."
            width="min(620px, 100vw)"
            footer={
                <>
                    <span className="sp-spacer" />
                    <button type="button" className="sp-btn" onClick={close} disabled={busy}>Готово</button>
                </>
            }
        >
            <div className="tx-form" style={{ gap: 24 }}>
                <section className="tx-form" aria-labelledby="excel-download-heading">
                    <h4 id="excel-download-heading" className="sp-row-label">1. Скачайте книгу</h4>
                    <p className="sp-row-hint">Откройте её в Excel для Windows. После подключения синхронизация работает, пока книга открыта.</p>
                    <div>
                        <button type="button" className="sp-btn" onClick={() => void downloadWorkbook()} disabled={downloading}>
                            <Download size={15} />
                            {downloading ? 'Скачиваем…' : 'Скачать книгу .xlsm'}
                        </button>
                    </div>
                    {downloadError && <p className="tx-field-error" role="alert">{downloadError}</p>}
                </section>

                <section className="tx-form" aria-labelledby="excel-connect-heading">
                    <h4 id="excel-connect-heading" className="sp-row-label">2. Подключите Excel</h4>
                    {!issued && (
                        <form className="tx-form" onSubmit={(event) => void createGrant(event)}>
                            <div className="tx-field">
                                <label className="tx-field-label" htmlFor="excel-grant-name">Название книги</label>
                                <input id="excel-grant-name" className="tx-field-input" value={name} maxLength={120} required
                                    onChange={(event) => setName(event.target.value)} disabled={busy} autoComplete="off" />
                            </div>
                            <div className="tx-field">
                                <label className="tx-field-label" htmlFor="excel-grant-expiry">Срок доступа</label>
                                <select id="excel-grant-expiry" className="tx-field-select" value={expiresInDays}
                                    onChange={(event) => setExpiresInDays(Number(event.target.value))} disabled={busy}>
                                    <option value={30}>30 дней</option>
                                    <option value={90}>90 дней</option>
                                    <option value={180}>180 дней</option>
                                    <option value={365}>1 год</option>
                                </select>
                            </div>
                            <p className="sp-row-hint">Для каждой книги создавайте отдельное подключение. Его можно отозвать в любой момент.</p>
                            <div>
                                <button type="submit" className="sp-btn sp-btn-primary" disabled={busy || loading || Boolean(listError) || !name.trim()}>
                                    {creating ? 'Создаём…' : 'Создать подключение'}
                                </button>
                            </div>
                        </form>
                    )}
                    {connection ? (
                        <div className="tx-form">
                            <p className="sp-row-hint">В книге нажмите «Подключить» и вставьте адрес сервера и токен. Токен показывается только сейчас: после закрытия этой панели его нельзя посмотреть снова.</p>
                            {(['server', 'token'] as const).map((field) => (
                                <div className="tx-field" key={field}>
                                    <label className="tx-field-label" htmlFor={`excel-connect-${field}`}>
                                        {field === 'server' ? 'Адрес сервера' : 'Токен подключения'}
                                    </label>
                                    <div style={{ display: 'flex', gap: 8 }}>
                                        <input id={`excel-connect-${field}`} className="tx-field-input" style={{ flex: 1, minWidth: 0 }} readOnly
                                            value={field === 'server' ? connection.server_url : connection.token}
                                            autoComplete="off" spellCheck={false} onFocus={(event) => event.currentTarget.select()} />
                                        <button type="button" className="sp-btn" onClick={() => void copy(field)}
                                            aria-label={field === 'server' ? 'Скопировать адрес сервера' : 'Скопировать токен'}>
                                            {copied === field ? <Check size={15} /> : <Copy size={15} />}
                                            {copied === field ? 'Скопировано' : 'Копировать'}
                                        </button>
                                    </div>
                                </div>
                            ))}
                            <p className="sp-row-hint">Токен даёт доступ к вашим чекам. Вставляйте его только в свою книгу.</p>
                            {copyError && <p className="tx-field-error" role="alert">{copyError}</p>}
                        </div>
                    ) : issued ? (
                        <p className="sp-row-hint">Токен больше не показывается. Для нового подключения закройте эту панель и откройте её снова.</p>
                    ) : null}
                    {error && <p className="tx-field-error" role="alert">{error}</p>}
                </section>

                <section className="tx-form" aria-labelledby="excel-password-heading">
                    <h4 id="excel-password-heading" className="sp-row-label">3. Защитите книгу паролем</h4>
                    <p className="sp-row-hint">
                        На новом компьютере книга показывает только лист «Доступ», пока не введён пароль. После верного пароля
                        чеки загружаются, а компьютер запоминается и больше пароль не спрашивает. Копия файла на другом
                        компьютере снова попросит пароль.
                    </p>
                    {activationsError && <p className="tx-field-error" role="alert">{activationsError}</p>}
                    {!loading && !listError && books.length === 0 && (
                        <p className="sp-row-hint">Пароль можно задать после первого подключения книги.</p>
                    )}
                    {books.map((book) => {
                        const activation = activations[book.id];
                        const lockedUntil = activation?.locked_until ?? null;
                        const saving = passwordBook === book.id;
                        const draft = passwords[book.id] ?? '';
                        const inputId = `excel-password-${book.id}`;
                        return (
                            <form key={book.id} className="tx-form" style={{ borderTop: '1px solid var(--border)', paddingTop: 12 }}
                                onSubmit={(event) => void saveActivationPassword(event, book)}>
                                <div className="sp-row-label" style={{ overflowWrap: 'anywhere' }}>{book.name}</div>
                                <div className="sp-row-hint">
                                    {activation
                                        ? `Пароль задан${activation.updated_at ? ` ${formatDate(activation.updated_at)}` : ''}.`
                                        : 'Пароль не задан: на новом компьютере книгу открыть нельзя.'}
                                </div>
                                {lockedUntil && (
                                    <div className="sp-row-hint">Слишком много неверных попыток: ввод пароля заблокирован до {formatDate(lockedUntil)}. Новый пароль снимает блокировку.</div>
                                )}
                                <div className="tx-field">
                                    <label className="tx-field-label" htmlFor={inputId}>{activation ? 'Новый пароль' : 'Пароль'}</label>
                                    <input id={inputId} className="tx-field-input" type="password" value={draft}
                                        minLength={MIN_PASSWORD} maxLength={128} autoComplete="new-password" spellCheck={false}
                                        disabled={busy || loading} placeholder={`Не короче ${MIN_PASSWORD} символов`}
                                        onChange={(event) => {
                                            const value = event.target.value;
                                            setPasswords((current) => ({ ...current, [book.id]: value }));
                                            if (passwordSaved === book.id) setPasswordSaved(null);
                                        }} />
                                </div>
                                <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                                    <button type="submit" className="sp-btn sp-btn-primary" disabled={busy || loading || draft.trim().length < MIN_PASSWORD}>
                                        <KeyRound size={14} /> {saving ? 'Сохраняем…' : activation ? 'Сменить пароль' : 'Задать пароль'}
                                    </button>
                                    {activation && (
                                        <button type="button" className="sp-btn" disabled={busy || loading}
                                            onClick={() => void removeActivationPassword(book)} aria-label={`Убрать пароль: ${book.name}`}>
                                            Убрать пароль
                                        </button>
                                    )}
                                </div>
                                {passwordSaved === book.id && (
                                    <p className="sp-row-hint" role="status">Пароль сохранён. Уже активированные компьютеры продолжают работать.</p>
                                )}
                                {passwordError?.book === book.id && <p className="tx-field-error" role="alert">{passwordError.message}</p>}
                            </form>
                        );
                    })}
                </section>

                <section className="tx-form" aria-labelledby="excel-grants-heading">
                    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
                        <h4 id="excel-grants-heading" className="sp-row-label">Ваши подключения</h4>
                        <button type="button" className="sp-btn sp-btn-ghost" disabled={loading || busy}
                            onClick={() => void loadGrants(sessionRef.current)} aria-label="Обновить список подключений">
                            <RefreshCw size={14} /> Обновить
                        </button>
                    </div>
                    {loading && <p className="sp-row-hint" role="status">Загружаем подключения…</p>}
                    {listError && <p className="tx-field-error" role="alert">{listError}</p>}
                    {!loading && !listError && grants.length === 0 && <p className="sp-row-hint">Подключённых книг пока нет.</p>}
                    {grants.map((grant) => {
                        const expired = grant.expires_at !== null && new Date(grant.expires_at).getTime() <= Date.now();
                        const status = grant.revoked_at ? 'Отозвано' : expired ? 'Срок истёк' : grant.workbook_id ? 'Подключено' : 'Ожидает подключения';
                        return (
                            <div key={grant.id} style={{ borderTop: '1px solid var(--border)', paddingTop: 12, display: 'flex', gap: 12, justifyContent: 'space-between' }}>
                                <div style={{ minWidth: 0 }}>
                                    <div className="sp-row-label" style={{ overflowWrap: 'anywhere' }}>{grant.name}</div>
                                    <div className="sp-row-hint">{status} · {grant.expires_at === null ? 'Бессрочно' : `до ${formatDate(grant.expires_at)}`}</div>
                                    {grant.device_bound && (
                                        <div className="sp-row-hint">Работает только на компьютере «{grant.device_name || 'без имени'}». Отзовите, чтобы он снова попросил пароль.</div>
                                    )}
                                    <div className="sp-row-hint">Последняя связь: {formatDate(grant.last_used_at)}</div>
                                </div>
                                {!grant.revoked_at && (
                                    <button type="button" className="sp-btn" style={{ alignSelf: 'flex-start', flexShrink: 0 }}
                                        disabled={busy || loading} onClick={() => void revokeGrant(grant)} aria-label={`Отозвать доступ: ${grant.name}`}>
                                        <Unplug size={14} /> {revokingId === grant.id ? 'Отзываем…' : 'Отозвать'}
                                    </button>
                                )}
                            </div>
                        );
                    })}
                </section>
            </div>
        </Sheet>
    );
}
