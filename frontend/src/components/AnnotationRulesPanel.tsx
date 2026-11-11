/**
 * The rule engine, which until now had no interface at all.
 *
 * Rules learn `оператор → значение` from rows a human wrote or the customer's
 * workbook supplied, and fill in the same answer for receipts that arrive later.
 * Four endpoints existed and nothing in the app ever called them — recompute,
 * apply and coverage were reachable only by curl or through the AI agent.
 *
 * Applying is safe to repeat: it only writes fields that are empty or were
 * themselves written by automation, so hand-entered bookkeeping is never
 * disturbed. The preview runs first for exactly that reassurance.
 */
import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Loader2, RefreshCw, Wand2 } from 'lucide-react';

import { transactionsApi } from '../services/api';
import { useToast } from './Toast';

type ApplySummary = {
    scanned: number;
    updated: number;
    changed_fields: number;
    dry_run: boolean;
};

export function AnnotationRulesPanel() {
    const queryClient = useQueryClient();
    const { showToast } = useToast();
    const [preview, setPreview] = useState<ApplySummary | null>(null);

    const coverageQuery = useQuery({
        queryKey: ['annotation-rules-coverage'],
        queryFn: () => transactionsApi.getAnnotationRulesCoverage(),
        staleTime: 60 * 1000,
    });

    const rulesQuery = useQuery({
        queryKey: ['annotation-rules'],
        queryFn: () => transactionsApi.getAnnotationRules({ page_size: 1 }),
        staleTime: 60 * 1000,
    });

    const refresh = () => {
        queryClient.invalidateQueries({ queryKey: ['annotation-rules-coverage'] });
        queryClient.invalidateQueries({ queryKey: ['annotation-rules'] });
        queryClient.invalidateQueries({ queryKey: ['transactions'] });
    };

    const recomputeMutation = useMutation({
        mutationFn: () => transactionsApi.recomputeAnnotationRules(),
        onSuccess: (data: any) => {
            refresh();
            showToast('success', `Правил активно: ${data?.total_enabled ?? 0} (новых ${data?.created ?? 0})`);
        },
        onError: () => showToast('error', 'Не удалось пересчитать правила'),
    });

    const applyMutation = useMutation({
        mutationFn: (dryRun: boolean) => transactionsApi.applyAnnotationRules({ dry_run: dryRun }),
        onSuccess: (data: any, dryRun) => {
            if (dryRun) {
                setPreview(data);
                showToast('info', `Будет дозаполнено строк: ${data?.updated ?? 0}`);
                return;
            }
            setPreview(null);
            refresh();
            showToast('success', `Дозаполнено строк: ${data?.updated ?? 0}`);
        },
        onError: () => showToast('error', 'Не удалось применить правила'),
    });

    const coverage = coverageQuery.data as any;
    const busy = recomputeMutation.isPending || applyMutation.isPending;

    return (
        <section className="settings-card">
            <header className="settings-card-head">
                <h3>Правила разметки</h3>
                <p className="settings-card-sub">
                    Правило запоминает, какое значение вы обычно ставите этому оператору, и подставляет
                    его новым чекам. Ручные правки не затрагиваются.
                </p>
            </header>

            <div className="settings-card-body">
                {coverageQuery.isLoading ? (
                    <Loader2 size={16} className="animate-spin" />
                ) : (
                    <dl className="settings-stats">
                        <div>
                            <dt>Активных правил</dt>
                            <dd>{(rulesQuery.data as any)?.total ?? '—'}</dd>
                        </div>
                        {(['article', 'person', 'details'] as const).map((field) => {
                            const block = coverage?.[field];
                            const label = { article: 'Статья', person: 'Лицо', details: 'Подробности' }[field];
                            return (
                                <div key={field}>
                                    <dt>{label} · предсказуемо</dt>
                                    <dd>{block?.coverage?.['0.9']?.pct != null ? `${block.coverage['0.9'].pct}%` : '—'}</dd>
                                </div>
                            );
                        })}
                    </dl>
                )}

                {preview ? (
                    <p className="settings-hint">
                        Предпросмотр: просмотрено {preview.scanned}, будет изменено строк {preview.updated}
                        {' '}(полей {preview.changed_fields}).
                    </p>
                ) : null}

                <div className="settings-actions">
                    <button
                        type="button"
                        className="rf-btn"
                        onClick={() => recomputeMutation.mutate()}
                        disabled={busy}
                    >
                        <RefreshCw size={14} />
                        Пересчитать правила
                    </button>
                    <button
                        type="button"
                        className="rf-btn"
                        onClick={() => applyMutation.mutate(true)}
                        disabled={busy}
                    >
                        Предпросмотр
                    </button>
                    <button
                        type="button"
                        className="rf-btn rf-btn-primary"
                        onClick={() => applyMutation.mutate(false)}
                        disabled={busy || !preview}
                        title={preview ? undefined : 'Сначала посмотрите предпросмотр'}
                    >
                        <Wand2 size={14} />
                        Дозаполнить
                    </button>
                </div>
            </div>
        </section>
    );
}
