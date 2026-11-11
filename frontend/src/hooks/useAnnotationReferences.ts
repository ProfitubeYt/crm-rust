import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { referenceApi, type AnnotationArticle, type AnnotationPerson } from '../services/api';
import type { SelectOption } from '../components/EditableCell';

/**
 * Article and person dictionaries backing the annotation columns.
 *
 * Both sets are tiny (14 articles, ~11 people) and change rarely, so they are
 * cached aggressively -- the grid re-renders often and must not refetch them.
 */
export function useAnnotationReferences() {
    const articlesQuery = useQuery<AnnotationArticle[]>({
        queryKey: ['annotation-articles'],
        queryFn: () => referenceApi.getAnnotationArticles(),
        staleTime: 10 * 60 * 1000,
        gcTime: 30 * 60 * 1000,
        retry: 1,
    });

    const personsQuery = useQuery<AnnotationPerson[]>({
        queryKey: ['annotation-persons'],
        queryFn: () => referenceApi.getAnnotationPersons(),
        staleTime: 10 * 60 * 1000,
        gcTime: 30 * 60 * 1000,
        retry: 1,
    });

    const articles = articlesQuery.data ?? [];
    const persons = personsQuery.data ?? [];

    /** code -> label, for rendering "р1=ФЗП,ФМП,МП" in a cell. */
    const articleLabels = useMemo(() => {
        const map: Record<string, string> = {};
        for (const item of articles) map[item.code] = item.label;
        return map;
    }, [articles]);

    const articleOptions = useMemo<SelectOption[]>(
        () => articles.map((item) => ({ value: item.code, label: `${item.code}=${item.label}` })),
        [articles],
    );

    const personOptions = useMemo<SelectOption[]>(
        () => persons.map((item) => ({
            value: item.code,
            label: item.full_name ? `${item.code} — ${item.full_name}` : item.code,
        })),
        [persons],
    );

    return { articles, persons, articleLabels, articleOptions, personOptions };
}
