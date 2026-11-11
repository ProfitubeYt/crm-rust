import type ExcelJS from 'exceljs';
import { AccessScope, Transaction, securityApi } from './api';
import { formatDate, formatTime, formatDateTime } from '../utils/dateTimeFormatters';
import { buildClientWorkbook } from './excelClientWorkbook';

type Alignment = 'left' | 'center' | 'right';

export type ExportColumn = {
    id: string;
    header: string;
    widthPx?: number;
    textAlign?: Alignment;
};

type StyleMap = Record<string, { backgroundColor?: string; textAlign?: Alignment; fontWeight?: string }>;

const HEADER_FILL = 'FFE5E7EB'; // gray-200
const BORDER_SIDE = { style: 'thin', color: { argb: 'FFCBD5E1' } } as const; // slate-300
const DATA_FONT_SIZE = 10;
const HEADER_FONT_SIZE = 10;

/** Alignment mapping by column ID (not position — safe with reorder/hide) */
const COLUMN_ALIGNMENT: Record<string, Alignment> = {
    row_number: 'center',
    operation_number: 'center',
    date_time: 'center',
    transaction_date: 'center',
    time: 'center',
    day: 'center',
    operator_raw: 'left',
    application_mapped: 'center',
    receiver_name: 'left',
    receiver_card: 'center',
    amount: 'right',
    balance_after: 'right',
    card_last_4: 'center',
    is_p2p: 'center',
    transaction_type: 'center',
    currency: 'center',
    source_type: 'center',
    parsing_method: 'center',
    parsing_confidence: 'center',
    annotation_details: 'left',
    annotation_person_code: 'center',
    annotation_period: 'center',
    annotation_article_code: 'left',
};

/**
 * Fixed widths for columns with known, constant-format data.
 * Values are in Excel character units (≈ number of characters in Calibri 11pt).
 */
const FIXED_WIDTHS: Record<string, number> = {
    row_number: 5,           // "№" → "9999"
    operation_number: 9,     // "№ опер." → "999999"
    date_time: 17,           // "Дата и Время" → "30.01.2026 11:40"
    transaction_date: 11,    // "Дата" → "30.01.2026"
    time: 6,                 // "Время" → "11:40"
    day: 5,                  // "День" → "пт"
    card_last_4: 5,          // "ПК" → "1234"
    is_p2p: 4,               // "П2П" → "1"
    transaction_type: 11,    // "Тип" → "Пополнение"
    currency: 7,             // "Валюта" → "UZS"
    source_type: 9,          // "Источник" → "Телеграм"
    parsing_method: 6,       // → "Regex"
    parsing_confidence: 9,   // → "100%"
    annotation_person_code: 7,   // "Лицо" → "ДиК"
    annotation_period: 9,        // "Период" → "03.2026"
};

const colorToARGB = (color?: string) => {
    if (!color || color === 'transparent') return undefined;
    const hex = color.trim();
    if (hex.startsWith('#')) {
        const clean = hex.replace('#', '');
        const full = clean.length === 3
            ? clean.split('').map(c => c + c).join('')
            : clean.padEnd(6, '0');
        return `FF${full.toUpperCase()}`;
    }
    const rgbaMatch = hex.match(/rgba?\((\d+),\s*(\d+),\s*(\d+)(?:,\s*([0-9.]+))?\)/i);
    if (rgbaMatch) {
        const r = parseInt(rgbaMatch[1], 10);
        const g = parseInt(rgbaMatch[2], 10);
        const b = parseInt(rgbaMatch[3], 10);
        const a = rgbaMatch[4] !== undefined ? Math.round(parseFloat(rgbaMatch[4]) * 255) : 255;
        const toHex = (v: number) => v.toString(16).padStart(2, '0').toUpperCase();
        return `${toHex(a)}${toHex(r)}${toHex(g)}${toHex(b)}`;
    }
    return undefined;
};

const formatExcelValue = (
    row: Transaction,
    columnId: string,
    rowIndex: number,
    totalRows: number,
    operationNumberDescending: boolean,
    articleLabels?: Record<string, string>,
) => {
    const value = (row as unknown as Record<string, unknown>)[columnId];
    const txDate = row.transaction_date ? new Date(row.transaction_date) : null;

    if (columnId === 'row_number') {
        return rowIndex + 1;
    }
    if (columnId === 'operation_number') {
        return operationNumberDescending ? Math.max(totalRows - rowIndex, 0) : rowIndex + 1;
    }
    if (columnId === 'date_time') return txDate ? formatDateTime(txDate) : '';
    if (columnId === 'transaction_date') return txDate ? formatDate(txDate) : '';
    if (columnId === 'time') return txDate ? formatTime(txDate) : '';
    if (columnId === 'day') {
        const days = ['вс', 'пн', 'вт', 'ср', 'чт', 'пт', 'сб'];
        return txDate ? days[txDate.getDay()] : '';
    }
    if (columnId === 'amount' || columnId === 'balance_after') {
        const num = value !== undefined && value !== null ? parseFloat(String(value)) : NaN;
        return Number.isNaN(num) ? '' : Math.abs(num);
    }
    if (columnId === 'parsing_confidence') {
        if (value === null || value === undefined) return '';
        return `${Math.round(Number(value) * 100)}%`;
    }
    if (columnId === 'is_p2p') return value ? 1 : '';
    if (columnId === 'transaction_type') {
        const map: Record<string, string> = {
            DEBIT: 'Списание',
            CREDIT: 'Пополнение',
            CONVERSION: 'Конверсия',
            REVERSAL: 'Отмена',
        };
        return row.transaction_type_display || map[String(value)] || String(value ?? '');
    }
    if (columnId === 'source_type') {
        const sourceLabels: Record<string, string> = {
            TELEGRAM: 'Телеграм',
            SMS: 'СМС',
            MANUAL: 'Ручной',
        };
        return row.source_display || sourceLabels[row.source_channel as string] || '';
    }
    if (columnId === 'parsing_method') {
        if (!value) return '';
        if (String(value).startsWith('REGEX')) return 'Regex';
        return String(value);
    }
    if (columnId === 'annotation_period') {
        // Stored as the first day of a month; the month is the meaning.
        const raw = value ? String(value) : '';
        if (!raw) return '';
        const [year, month] = raw.split('-');
        return year && month ? `${month}.${year}` : raw;
    }
    if (columnId === 'annotation_article_code') {
        const code = value ? String(value) : '';
        if (!code) return '';
        const label = articleLabels?.[code];
        return label ? `${code}=${label}` : code;
    }
    if (columnId === 'receiver_name') return value ?? '';
    if (columnId === 'receiver_card') return value ?? '';
    return value ?? '';
};

const isTransactionWithinScope = (row: Transaction, scope?: AccessScope | null): boolean => {
    if (!scope) return true;
    const txDate = row.transaction_date ? new Date(row.transaction_date) : null;
    if (!txDate || Number.isNaN(txDate.getTime())) return false;

    const scopeYears = scope.years || [];
    if (scopeYears.length && !scopeYears.includes(txDate.getFullYear())) {
        return false;
    }

    if (scope.period_start) {
        const from = new Date(scope.period_start);
        if (!Number.isNaN(from.getTime()) && txDate < from) {
            return false;
        }
    }
    if (scope.period_end) {
        const to = new Date(scope.period_end);
        if (!Number.isNaN(to.getTime()) && txDate > to) {
            return false;
        }
    }
    return true;
};

const getNumberFormat = (columnId: string) => {
    if (columnId === 'amount' || columnId === 'balance_after') return '#,##0.00';
    if (columnId === 'date_time') return 'yyyy.mm.dd hh:mm';
    if (columnId === 'transaction_date') return 'yyyy.mm.dd';
    if (columnId === 'time') return 'hh:mm';
    return undefined;
};

type ExportOptions = {
    rows: Transaction[];
    columns: ExportColumn[];
    columnStyles?: StyleMap;
    cellStyles?: StyleMap;
    fileName?: string;
    includeAlternating?: boolean;
    operationNumberDescending?: boolean;
    operationNumberTotal?: number;
    /**
     * 'single' keeps the historical one-sheet export.
     * 'client' reproduces the customer's workbook: their own file as the
     * template, one sheet per source, their colours and their emoji.
     */
    layout?: 'single' | 'client';
    /** code -> label, so the article column can render "р1=ФЗП,ФМП,МП". */
    articleLabels?: Record<string, string>;
};

export const exportTransactionsToExcel = async (options: ExportOptions) => {
    const {
        rows,
        columns,
        columnStyles = {},
        cellStyles = {},
        fileName = 'transactions.xlsx',
        includeAlternating = false,
        operationNumberDescending = false,
        operationNumberTotal,
        layout = 'single',
        articleLabels,
    } = options;
    let exportRows = rows;
    try {
        const [lockResponse, scopeStatus] = await Promise.all([
            securityApi.getLockedPeriods(),
            securityApi.getStatus(),
        ]);
        const periods = lockResponse?.periods || [];
        const currentScope = scopeStatus?.current_scope || null;

        exportRows = rows.filter((row) => isTransactionWithinScope(row, currentScope));

        if (periods.length) {
            exportRows = exportRows.filter((row) => {
                const txDate = row.transaction_date ? new Date(row.transaction_date) : null;
                if (!txDate || Number.isNaN(txDate.getTime())) return true;
                const txDateOnly = new Date(txDate.getFullYear(), txDate.getMonth(), txDate.getDate()).getTime();
                return !periods.some((period) => {
                    const from = new Date(period.date_from);
                    const to = new Date(period.date_to);
                    if (Number.isNaN(from.getTime()) || Number.isNaN(to.getTime())) return false;
                    const fromOnly = new Date(from.getFullYear(), from.getMonth(), from.getDate()).getTime();
                    const toOnly = new Date(to.getFullYear(), to.getMonth(), to.getDate()).getTime();
                    return txDateOnly >= fromOnly && txDateOnly <= toOnly;
                });
            });
        }
    } catch {
        // Export should still work if lock metadata endpoint is temporarily unavailable.
    }

    // Pulled in on demand so the ~900KB exceljs bundle never lands in the
    // startup chunk — only when a user actually triggers an export.
    const { default: ExcelJSModule } = await import('exceljs');

    const buildPlainSheet = (workbook: ExcelJS.Workbook, sheetName: string, sheetRows: Transaction[]) => {
        const sheet = workbook.addWorksheet(sheetName, {
            properties: { defaultRowHeight: 15 },
            pageSetup: { fitToPage: true },
        });

        sheet.columns = columns.map(col => ({ header: col.header, key: col.id, width: 10 }));
        sheet.views = [{ state: 'frozen', ySplit: 1 }];
        sheet.autoFilter = { from: { row: 1, column: 1 }, to: { row: 1, column: columns.length } };

        const headerRow = sheet.getRow(1);
        headerRow.font = { bold: true, size: HEADER_FONT_SIZE, color: { argb: 'FF0F172A' } };
        headerRow.fill = { type: 'pattern', pattern: 'solid', fgColor: { argb: HEADER_FILL } };
        headerRow.alignment = { vertical: 'middle', wrapText: false };
        headerRow.height = 18;
        headerRow.eachCell((cell, colNumber) => {
            cell.border = { top: BORDER_SIDE, left: BORDER_SIDE, bottom: BORDER_SIDE, right: BORDER_SIDE } as ExcelJS.Borders;
            const colId = columns[colNumber - 1]?.id;
            cell.alignment = {
                vertical: 'middle',
                horizontal: (colId ? COLUMN_ALIGNMENT[colId] : undefined) || 'center',
                wrapText: false,
            };
        });

        sheetRows.forEach((row, rowIdx) => {
            const excelRow = sheet.addRow(
                columns.map(col => formatExcelValue(
                    row, col.id, rowIdx,
                    operationNumberTotal ?? sheetRows.length,
                    operationNumberDescending, articleLabels,
                )),
            );
            excelRow.height = 15;
            const isEven = rowIdx % 2 === 1;
            columns.forEach((col, colIdx) => {
                const excelCell = excelRow.getCell(colIdx + 1);
                const colStyle = columnStyles[col.id] || {};
                const cellStyle = cellStyles[`${row.id}:${col.id}`] || {};
                const isBold = colStyle.fontWeight === 'bold' || cellStyle.fontWeight === 'bold';
                excelCell.font = { size: DATA_FONT_SIZE, ...(isBold ? { bold: true } : {}) };

                const bg = colorToARGB(cellStyle.backgroundColor || colStyle.backgroundColor);
                if (bg) {
                    excelCell.fill = { type: 'pattern', pattern: 'solid', fgColor: { argb: bg } };
                } else if (includeAlternating && isEven) {
                    excelCell.fill = { type: 'pattern', pattern: 'solid', fgColor: { argb: 'FFF8FAFC' } };
                }
                excelCell.alignment = {
                    horizontal: COLUMN_ALIGNMENT[col.id] || 'left', vertical: 'middle', wrapText: false,
                };
                const numFmt = getNumberFormat(col.id);
                if (numFmt) excelCell.numFmt = numFmt;
                excelCell.border = { top: BORDER_SIDE, left: BORDER_SIDE, bottom: BORDER_SIDE, right: BORDER_SIDE } as ExcelJS.Borders;
            });
        });

        sheet.columns.forEach((col, idx) => {
            const colId = columns[idx]?.id;
            const fixed = colId ? FIXED_WIDTHS[colId] : undefined;
            if (fixed) { col.width = fixed; return; }
            const isNumericCol = colId === 'amount' || colId === 'balance_after';
            let maxLen = String(col.header ?? '').length;
            sheet.getColumn(idx + 1).eachCell({ includeEmpty: false }, (cell, rowNumber) => {
                if (rowNumber === 1) return;
                const val = cell.value;
                const len = isNumericCol && typeof val === 'number'
                    ? new Intl.NumberFormat('ru-RU', { minimumFractionDigits: 2, maximumFractionDigits: 2 })
                        .format(Math.abs(val)).length
                    : (val === null || val === undefined ? 0 : String(val).length);
                if (len > maxLen) maxLen = len;
            });
            const computed = Math.min(maxLen + 1, 40);
            col.width = isNumericCol ? Math.max(computed, 14) : Math.max(computed, 8);
        });
    };

    let workbook: ExcelJS.Workbook;
    if (layout === 'client') {
        // The customer's own file is the template; see excelClientWorkbook.ts.
        workbook = await buildClientWorkbook(ExcelJSModule, exportRows, {
            descending: operationNumberDescending,
            articleLabels,
        });
    } else {
        workbook = new ExcelJSModule.Workbook();
        buildPlainSheet(workbook, 'Транзакции', exportRows);
    }

    // --- Write & download ---
    const buffer = await workbook.xlsx.writeBuffer();
    const blob = new Blob([buffer], {
        type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
    });
    const url = window.URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = fileName;
    link.click();
    window.URL.revokeObjectURL(url);
};
