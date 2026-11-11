import React, { useCallback, useEffect, useRef, useState } from 'react';
import { transactionsApi, type AnnotationImportReport } from '../services/api';
import { useToast } from './Toast';

/**
 * Desktop-only panel for the client's bookkeeping workbook.
 *
 * The main process watches the file and reports changes; this component does
 * the upload, because the access token and the `X-System-Access` broker live on
 * the renderer side. Keeping the request here means authentication has exactly
 * one implementation.
 *
 * An import always runs as a dry run first. It is committed automatically only
 * when nothing conflicts with hand-entered values — otherwise a person decides.
 */
export const AnnotationImportPanel: React.FC = () => {
    const { showToast } = useToast();
    const [state, setState] = useState<AnnotationWatcherState | null>(null);
    const [busy, setBusy] = useState(false);
    const [report, setReport] = useState<AnnotationImportReport | null>(null);
    const [pendingConflicts, setPendingConflicts] = useState<string | null>(null);
    const runningRef = useRef(false);

    const api = typeof window !== 'undefined' ? window.electronAPI?.annotations : undefined;

    const refresh = useCallback(async () => {
        if (!api) return;
        try {
            setState(await api.getState());
        } catch {
            // Watcher not available; the panel simply stays idle.
        }
    }, [api]);

    const runImport = useCallback(async (commit: boolean) => {
        if (!api || runningRef.current) return;
        runningRef.current = true;
        setBusy(true);
        setPendingConflicts(null);
        try {
            const file = await api.readFile();
            const blob = new Blob([file.content as unknown as BlobPart], {
                type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
            });

            const dry = await transactionsApi.importAnnotations(blob, {
                dryRun: true,
                source: 'electron_watcher',
                fileName: file.fileName,
            });
            setReport(dry.report);

            const conflicts = dry.report.counts.conflicts;
            // Auto-commit only when nothing a human typed would be touched.
            if (!commit && conflicts > 0) {
                setPendingConflicts(
                    `Найдено расхождений с ручной правкой: ${conflicts}. Импорт не применён.`,
                );
                return;
            }
            if (dry.report.matched === 0) {
                showToast('warning', 'Ни одна строка книги не сопоставилась с транзакциями.');
                return;
            }

            const applied = await transactionsApi.importAnnotations(blob, {
                dryRun: false,
                source: 'electron_watcher',
                fileName: file.fileName,
                overwriteManual: commit,
            });
            setReport(applied.report);
            await api.markImported(file.sha256);
            await refresh();
            showToast(
                'success',
                `Разметка обновлена: ${applied.report.applied} транзакций.`,
            );
        } catch (error: any) {
            showToast('error', error?.response?.data?.detail || error?.message || 'Импорт не удался.');
        } finally {
            runningRef.current = false;
            setBusy(false);
        }
    }, [api, refresh, showToast]);

    useEffect(() => { void refresh(); }, [refresh]);

    useEffect(() => {
        if (!api?.onFileChanged) return;
        return api.onFileChanged(() => { void runImport(false); });
    }, [api, runImport]);

    if (!api) return null;

    return (
        <section className="settings-card">
            <header className="settings-card-header">
                <h3>Книга учёта</h3>
                <p className="settings-card-subtitle">
                    Подробности, лицо, период и статья подтягиваются из вашего Excel-файла.
                    Программа следит за ним и обновляет разметку после каждого сохранения.
                </p>
            </header>

            <div className="settings-row">
                <div>
                    <div className="settings-row-title">
                        {state?.fileName || 'Файл не выбран'}
                    </div>
                    {state?.lastRunAt && (
                        <div className="settings-row-subtitle">
                            Последняя загрузка: {new Date(state.lastRunAt).toLocaleString('ru-RU')}
                        </div>
                    )}
                </div>
                <div className="settings-row-actions">
                    <button
                        type="button"
                        className="btn btn-secondary"
                        disabled={busy}
                        onClick={async () => { setState(await api.pickFile()); }}
                    >
                        Выбрать файл
                    </button>
                    {state?.configured && (
                        <button
                            type="button"
                            className="btn btn-secondary"
                            disabled={busy}
                            onClick={async () => { setState(await api.setEnabled(!state.enabled)); }}
                        >
                            {state.enabled ? 'Остановить слежение' : 'Следить за файлом'}
                        </button>
                    )}
                    {state?.configured && (
                        <button
                            type="button"
                            className="btn btn-primary"
                            disabled={busy}
                            onClick={() => { void runImport(false); }}
                        >
                            {busy ? 'Импорт…' : 'Загрузить сейчас'}
                        </button>
                    )}
                </div>
            </div>

            {pendingConflicts && (
                <div className="settings-note settings-note-warning">
                    <div>{pendingConflicts}</div>
                    <button
                        type="button"
                        className="btn btn-secondary"
                        disabled={busy}
                        onClick={() => { void runImport(true); }}
                    >
                        Применить, перезаписав ручные правки
                    </button>
                </div>
            )}

            {report && (
                <dl className="settings-stats">
                    <div><dt>Строк в книге</dt><dd>{report.row_count}</dd></div>
                    <div><dt>Сопоставлено</dt><dd>{report.matched}</dd></div>
                    <div><dt>Обновлено</dt><dd>{report.applied}</dd></div>
                    <div><dt>Без изменений</dt><dd>{report.skipped_same}</dd></div>
                    <div><dt>Не найдено</dt><dd>{report.counts.unmatched}</dd></div>
                    <div><dt>Неоднозначных</dt><dd>{report.counts.ambiguous}</dd></div>
                    <div><dt>Конфликтов</dt><dd>{report.counts.conflicts}</dd></div>
                </dl>
            )}
        </section>
    );
};
