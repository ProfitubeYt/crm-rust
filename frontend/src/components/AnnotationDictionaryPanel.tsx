/**
 * The bookkeeping dictionaries: `код → расшифровка` for articles and persons.
 *
 * These codes were reachable only as dropdown options inside the transactions
 * grid, so nobody could see the full list or say what a code meant — and the
 * server had no write route at all, leaving direct SQL as the only way to curate
 * them. That matters now that the importer registers unfamiliar codes on its
 * own: a code marked "новый" here is one the customer's workbook used and
 * nobody has labelled yet.
 */
import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Check, Loader2, Plus, Search, Trash2, X } from 'lucide-react';

import { referenceApi, type AnnotationArticle, type AnnotationPerson } from '../services/api';
import { useToast } from './Toast';
import { ConfirmDialog } from './motion/ConfirmDialog';

type Kind = 'articles' | 'persons';
type Draft = { code: string; label: string };

const EMPTY_DRAFT: Draft = { code: '', label: '' };

export function AnnotationDictionaryPanel() {
    const queryClient = useQueryClient();
    const { showToast } = useToast();

    const [kind, setKind] = useState<Kind>('articles');
    const [search, setSearch] = useState('');
    const [editingId, setEditingId] = useState<number | null>(null);
    const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT);
    const [adding, setAdding] = useState(false);
    const [confirmDelete, setConfirmDelete] = useState<{ id: number; code: string } | null>(null);

    const articlesQuery = useQuery({
        queryKey: ['annotation-articles', 'all'],
        queryFn: () => referenceApi.getAnnotationArticles({ include_inactive: true }),
    });
    const personsQuery = useQuery({
        queryKey: ['annotation-persons', 'all'],
        queryFn: () => referenceApi.getAnnotationPersons({ include_inactive: true }),
    });

    const invalidate = () => {
        // The grid dropdowns and the export labels read the active-only lists.
        queryClient.invalidateQueries({ queryKey: ['annotation-articles'] });
        queryClient.invalidateQueries({ queryKey: ['annotation-persons'] });
    };

    const failed = (error: any, fallback: string) => {
        const detail = error?.response?.data?.detail;
        const messages: Record<string, string> = {
            article_code_exists: 'Такой код статьи уже есть.',
            person_code_exists: 'Такой код лица уже есть.',
            code_required: 'Укажите код.',
        };
        showToast('error', messages[detail] || fallback);
    };

    const saveMutation = useMutation({
        mutationFn: async (payload: { id?: number; code: string; label: string }) => {
            const isArticle = kind === 'articles';
            if (payload.id) {
                return isArticle
                    ? referenceApi.updateAnnotationArticle(payload.id, { code: payload.code, label: payload.label })
                    : referenceApi.updateAnnotationPerson(payload.id, { code: payload.code, full_name: payload.label });
            }
            return isArticle
                ? referenceApi.createAnnotationArticle({ code: payload.code, label: payload.label })
                : referenceApi.createAnnotationPerson({ code: payload.code, full_name: payload.label });
        },
        onSuccess: () => {
            invalidate();
            setEditingId(null);
            setAdding(false);
            setDraft(EMPTY_DRAFT);
            showToast('success', 'Сохранено');
        },
        onError: (error) => failed(error, 'Не удалось сохранить'),
    });

    const deleteMutation = useMutation({
        mutationFn: (id: number) =>
            kind === 'articles'
                ? referenceApi.deleteAnnotationArticle(id)
                : referenceApi.deleteAnnotationPerson(id),
        onSuccess: () => {
            invalidate();
            setConfirmDelete(null);
            showToast('success', 'Удалено');
        },
        onError: (error) => failed(error, 'Не удалось удалить'),
    });

    const rows = useMemo(() => {
        const needle = search.trim().toLowerCase();
        const source: Array<{ id: number; code: string; label: string; isActive: boolean; isAuto: boolean; usage?: string }> =
            kind === 'articles'
                ? (articlesQuery.data || []).map((row: AnnotationArticle) => ({
                    id: row.id,
                    code: row.code,
                    label: row.label,
                    isActive: row.is_active,
                    isAuto: row.is_auto,
                    usage: row.direction === 'income' ? 'приход' : 'расход',
                }))
                : (personsQuery.data || []).map((row: AnnotationPerson) => ({
                    id: row.id,
                    code: row.code,
                    label: row.full_name || '',
                    isActive: row.is_active,
                    isAuto: row.is_auto,
                }));

        if (!needle) return source;
        return source.filter(
            (row) => row.code.toLowerCase().includes(needle) || row.label.toLowerCase().includes(needle),
        );
    }, [kind, search, articlesQuery.data, personsQuery.data]);

    const isLoading = kind === 'articles' ? articlesQuery.isLoading : personsQuery.isLoading;
    const unlabelled = rows.filter((row) => row.isAuto).length;

    const beginEdit = (row: { id: number; code: string; label: string }) => {
        setAdding(false);
        setEditingId(row.id);
        setDraft({ code: row.code, label: row.label });
    };

    const submit = () => {
        const code = draft.code.trim();
        if (!code) {
            showToast('error', 'Укажите код.');
            return;
        }
        saveMutation.mutate({ id: editingId ?? undefined, code, label: draft.label.trim() });
    };

    return (
        <>
            <div className="rf-toolbar">
                <div className="rf-chips" role="tablist" aria-label="Тип справочника">
                    <button
                        type="button"
                        role="tab"
                        aria-selected={kind === 'articles'}
                        className={`rf-chip${kind === 'articles' ? ' is-active' : ''}`}
                        onClick={() => { setKind('articles'); setEditingId(null); setAdding(false); }}
                    >
                        Статьи
                        <span className="rf-chip-count">{articlesQuery.data?.length ?? 0}</span>
                    </button>
                    <button
                        type="button"
                        role="tab"
                        aria-selected={kind === 'persons'}
                        className={`rf-chip${kind === 'persons' ? ' is-active' : ''}`}
                        onClick={() => { setKind('persons'); setEditingId(null); setAdding(false); }}
                    >
                        Лица
                        <span className="rf-chip-count">{personsQuery.data?.length ?? 0}</span>
                    </button>
                </div>

                <div className="rf-search">
                    <Search size={14} className="rf-search-icon" aria-hidden />
                    <input
                        type="text"
                        value={search}
                        onChange={(event) => setSearch(event.target.value)}
                        placeholder="Код или расшифровка"
                        className="rf-search-input"
                    />
                </div>

                <span className="rf-toolbar-spacer" />

                {unlabelled > 0 ? (
                    <span className="rf-chip" title="Коды пришли из книги клиента и ждут расшифровки">
                        Без расшифровки
                        <span className="rf-chip-count">{unlabelled}</span>
                    </span>
                ) : null}

                <button
                    type="button"
                    className="rf-btn rf-btn-primary"
                    onClick={() => { setAdding(true); setEditingId(null); setDraft(EMPTY_DRAFT); }}
                >
                    <Plus size={14} />
                    Добавить
                </button>
            </div>

            <div className="rf-table-wrap">
                <div className="rf-table-scroll">
                    <table className="rf-table density-standard">
                        <thead>
                            <tr>
                                <th style={{ width: 140 }}>Код</th>
                                <th>Расшифровка</th>
                                {kind === 'articles' ? <th style={{ width: 110 }}>Направление</th> : null}
                                <th style={{ width: 120 }}>Статус</th>
                                <th style={{ width: 96 }} />
                            </tr>
                        </thead>
                        <tbody>
                            {adding ? (
                                <tr>
                                    <td>
                                        <input
                                            autoFocus
                                            className="rf-search-input"
                                            value={draft.code}
                                            onChange={(event) => setDraft((prev) => ({ ...prev, code: event.target.value }))}
                                            placeholder={kind === 'articles' ? 'р1' : 'СТ'}
                                        />
                                    </td>
                                    <td colSpan={kind === 'articles' ? 3 : 2}>
                                        <input
                                            className="rf-search-input"
                                            value={draft.label}
                                            onChange={(event) => setDraft((prev) => ({ ...prev, label: event.target.value }))}
                                            placeholder={kind === 'articles' ? 'ФЗП,ФМП,МП' : 'Имя'}
                                            onKeyDown={(event) => { if (event.key === 'Enter') submit(); }}
                                        />
                                    </td>
                                    <td>
                                        <button type="button" className="rf-btn" onClick={submit} disabled={saveMutation.isPending}>
                                            <Check size={14} />
                                        </button>
                                        <button type="button" className="rf-btn" onClick={() => setAdding(false)}>
                                            <X size={14} />
                                        </button>
                                    </td>
                                </tr>
                            ) : null}

                            {isLoading ? (
                                <tr>
                                    <td colSpan={5}>
                                        <div className="rf-empty">
                                            <Loader2 size={18} className="animate-spin" />
                                        </div>
                                    </td>
                                </tr>
                            ) : rows.length === 0 && !adding ? (
                                <tr>
                                    <td colSpan={5}>
                                        <div className="rf-empty">
                                            <div className="rf-empty-title">Ничего не найдено</div>
                                        </div>
                                    </td>
                                </tr>
                            ) : (
                                rows.map((row) => (
                                    <tr key={row.id} style={row.isActive ? undefined : { opacity: 0.55 }}>
                                        <td>
                                            {editingId === row.id ? (
                                                <input
                                                    autoFocus
                                                    className="rf-search-input"
                                                    value={draft.code}
                                                    onChange={(event) => setDraft((prev) => ({ ...prev, code: event.target.value }))}
                                                />
                                            ) : (
                                                <code>{row.code}</code>
                                            )}
                                        </td>
                                        <td>
                                            {editingId === row.id ? (
                                                <input
                                                    className="rf-search-input"
                                                    value={draft.label}
                                                    onChange={(event) => setDraft((prev) => ({ ...prev, label: event.target.value }))}
                                                    onKeyDown={(event) => { if (event.key === 'Enter') submit(); }}
                                                />
                                            ) : (
                                                row.label || <span style={{ opacity: 0.5 }}>— нет расшифровки —</span>
                                            )}
                                        </td>
                                        {kind === 'articles' ? <td>{row.usage}</td> : null}
                                        <td>
                                            {!row.isActive
                                                ? 'скрыт'
                                                : row.isAuto
                                                    ? 'новый'
                                                    : 'готов'}
                                        </td>
                                        <td>
                                            {editingId === row.id ? (
                                                <>
                                                    <button type="button" className="rf-btn" onClick={submit} disabled={saveMutation.isPending}>
                                                        <Check size={14} />
                                                    </button>
                                                    <button type="button" className="rf-btn" onClick={() => setEditingId(null)}>
                                                        <X size={14} />
                                                    </button>
                                                </>
                                            ) : (
                                                <>
                                                    <button type="button" className="rf-btn" onClick={() => beginEdit(row)}>
                                                        Изм.
                                                    </button>
                                                    <button
                                                        type="button"
                                                        className="rf-btn"
                                                        onClick={() => setConfirmDelete({ id: row.id, code: row.code })}
                                                    >
                                                        <Trash2 size={14} />
                                                    </button>
                                                </>
                                            )}
                                        </td>
                                    </tr>
                                ))
                            )}
                        </tbody>
                    </table>
                </div>
            </div>

            <ConfirmDialog
                open={confirmDelete !== null}
                title="Удалить код?"
                // A code that is in use is hidden rather than removed: deleting it
                // would leave historical rows carrying a code that resolves to
                // nothing, and the grid would show bare codes again.
                description={`Код ${confirmDelete?.code ?? ''} будет удалён. Если он уже проставлен в транзакциях, он останется в базе и будет просто скрыт из списков.`}
                confirmLabel="Удалить"
                onConfirm={() => { if (confirmDelete) deleteMutation.mutate(confirmDelete.id); }}
                onClose={() => setConfirmDelete(null)}
                tone="danger"
            />
        </>
    );
}
