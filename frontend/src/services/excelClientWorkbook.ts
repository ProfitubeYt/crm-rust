/**
 * The customer's workbook, built in the browser from the same template the
 * server uses.
 *
 * Every export button goes through `GET /api/transactions/export` whenever the
 * backend is reachable; that is what the customer normally receives, and
 * `backend/services/excel_client_layout.py` is the authority on the layout.
 * This module exists for the one path that cannot reach it — a locked view or
 * the offline read-only mirror — and follows the Python module step for step:
 * the same template (inlined by `tools/build_excel_client_template.py`), the
 * same column map, the same figures in the header rows, the same numbering.
 *
 * Two things ExcelJS cannot do that openpyxl can, and how they are handled:
 *
 * - The customer's sheet view hides zeros, and ExcelJS has no way to write that
 *   flag. Wherever the server writes a `0` that their view would hide (an empty
 *   `Приложение`, a p2p flag, their row of zeros above the data) this writes a
 *   blank instead, so the file *looks* the same.
 * - ExcelJS cannot write the `duplicateValues` rule that tints repeated
 *   timestamps on their HUMO sheet, and what it writes instead Excel refuses to
 *   open, so that rule is dropped on load. The rest of the conditional
 *   formatting survives.
 */
import type ExcelJS from 'exceljs';
import type { Transaction } from './api';
import { EXCEL_CLIENT_TEMPLATE_BASE64 } from './excelClientTemplate';

/** The part of ExcelJS this needs: something that makes workbooks. */
type ExcelModule = { Workbook: new () => ExcelJS.Workbook };
type CellValue = string | number | Date | null;
type RowValues = Record<string, unknown>;

/** One sheet of the customer's workbook and where our values go in it. */
export type SheetTemplate = {
    /** Template sheet name; also the source whose sheet this is. */
    sheet: string;
    /** The source's name as their captions spell it (`Сборка HumoCard`). */
    token: string;
    /** Rows copied verbatim above the data. */
    headerRows: number;
    /** Template row whose styles the first data row takes. */
    firstRow: number;
    /** Template row whose styles every other data row takes. */
    bodyRow: number;
    /** Column letter -> key in the row values. */
    columns: Record<string, string>;
    /** Cell -> figure written into the header rows. */
    figures: Record<string, string>;
    /** Header cells whose text names the source. */
    captions: string[];
    /** What their `G`..`L` import columns hold, in this sheet's own order. */
    importOrder: string[];
};

/** Their hidden import group, identical on both sheets. */
export const IMPORT_COLUMNS: Record<string, string> = {
    A: 'line', B: 'message_no', C: 'header_no', D: 'service', E: 'date', F: 'time',
    G: 'line_1', H: 'line_2', I: 'line_3', J: 'line_4', K: 'line_5', L: 'line_6', M: 'weekday',
};
const IMPORT_LAST_COLUMN = 14; // N

export const CARDXABAR: SheetTemplate = {
    sheet: 'CardXabar',
    token: 'CardXabar',
    headerRows: 4, // title, header, counts, and their row of zeros
    firstRow: 5,
    bodyRow: 6,
    columns: {
        O: 'row_number', P: 'operation_number', Q: 'date_time', R: 'date_time_dedup',
        S: 'source', T: 'operation', U: 'operator', W: 'application', X: 'sign',
        Y: 'amount_text', Z: 'amount_value', AF: 'balance_text', AG: 'balance_value',
        AH: 'card', AI: 'p2p', AJ: 'p2p_confirmed',
        AK: 'details', AL: 'person', AM: 'period', AN: 'article',
    },
    figures: { B1: 'count', P3: 'count', T3: 'last_operation_at', Z3: 'total_amount', AU3: 'count' },
    captions: ['O1', 'Q3', 'U3'],
    importOrder: ['headline', 'operator', 'amount', 'card', 'when', 'balance'],
};

export const HUMO: SheetTemplate = {
    sheet: 'HUMO Card',
    token: 'HumoCard',
    headerRows: 3, // title, header, counts
    firstRow: 4,
    bodyRow: 5,
    columns: {
        O: 'row_number', P: 'operation_number', Q: 'date_time', R: 'date_time_dedup',
        S: 'source', T: 'operation', U: 'operator', W: 'application', X: 'sign',
        Y: 'amount_text', Z: 'amount_digits', AA: 'amount_length',
        AB: 'amount_billions', AC: 'amount_millions', AD: 'amount_thousands', AE: 'amount_units',
        AF: 'amount_value',
        AG: 'balance_text', AH: 'balance_digits', AI: 'balance_length',
        AJ: 'balance_billions', AK: 'balance_millions', AL: 'balance_thousands', AM: 'balance_units',
        AN: 'balance_value',
        AO: 'card', AP: 'p2p', AQ: 'p2p_confirmed',
        AR: 'details', AS: 'person', AT: 'period', AU: 'article',
    },
    figures: { B1: 'count', P3: 'count' },
    captions: ['O1', 'Q3'],
    // Their HUMO sheet heads these columns Операция / Сумма / Оператор, where
    // CardXabar heads them Операция / Оператор / Сумма.
    importOrder: ['headline', 'amount', 'operator', 'card', 'when', 'balance'],
};

/** In the order their workbook has them. */
export const SHEET_TEMPLATES: SheetTemplate[] = [CARDXABAR, HUMO];

/** The sheet the workbook opens on: every receipt, in the CardXabar layout. */
export const COMBINED_SHEET_NAME = 'Все транзакции';
// The title strip is three narrow columns wide; `Сборка: все транзакции` is cut
// off at the column-number strip, the bare name is not.
export const COMBINED_CAPTIONS: Record<string, string> = {
    O1: 'Все транзакции',
    Q3: '- число операций, все сервисы',
    U3: '- дата крайней операции',
};

/** A ten-thousandth of a day: the step their `(Без дубля)` column adds per repeated timestamp. */
export const DEDUP_STEP_MS = 8640;

/** Lines of one receipt carried into the hidden import group. */
const IMPORT_LINES_PER_MESSAGE = 12;

const EXCEL_MAX_ROWS = 1048576;

/** HUMO gets its own sheet; every other bank is laid out the CardXabar way. */
export function templateForSource(source: string): SheetTemplate {
    return (source || '').toUpperCase().includes('HUMO') ? HUMO : CARDXABAR;
}

function normalise(text: string): string {
    return (text || '').replace(/\s+/g, '').toLowerCase();
}

/** Whether this is the very source the template sheet was built for. */
export function isNativeSource(template: SheetTemplate, source: string): boolean {
    return normalise(source) === normalise(template.sheet);
}

/** Their sheet order: the banks their workbook already has, then the rest as seen. */
export function orderSources(sources: Iterable<string>): string[] {
    const seen = Array.from(new Set(sources));
    const ordered: string[] = [];
    for (const template of SHEET_TEMPLATES) {
        for (const source of seen) {
            if (!ordered.includes(source) && isNativeSource(template, source)) ordered.push(source);
        }
    }
    for (const source of seen) if (!ordered.includes(source)) ordered.push(source);
    return ordered;
}

/** Group rows by the source the customer thinks in — one sheet each. */
export function groupBySource(rows: Transaction[]): Map<string, Transaction[]> {
    const groups = new Map<string, Transaction[]>();
    for (const row of rows) {
        const key = (row.source_display || row.source_channel || 'Прочее').trim() || 'Прочее';
        const bucket = groups.get(key);
        if (bucket) bucket.push(row);
        else groups.set(key, [row]);
    }
    return groups;
}

/**
 * Excel sheet names: max 31 chars, no `: \ / ? * [ ]`, unique per workbook.
 *
 * Russian chat titles run long, so truncation alone collides — and ExcelJS
 * raises on a duplicate name at export time, in front of the user. The counter
 * suffix is what prevents that.
 */
export function sanitizeSheetName(raw: string, taken: Set<string>): string {
    let name = (raw || 'Лист').replace(/[:\\/?*[\]]/g, ' ').replace(/\s+/g, ' ').trim();
    if (!name) name = 'Лист';
    name = name.slice(0, 31);
    // Excel's other sheet-name rule: a name may not begin or end with an
    // apostrophe. ExcelJS throws on it, so the browser export produced no file.
    name = name.replace(/^'+|'+$/g, '') || 'Лист';

    if (!taken.has(name)) {
        taken.add(name);
        return name;
    }
    for (let i = 2; i < 1000; i += 1) {
        const suffix = ` (${i})`;
        const candidate = name.slice(0, 31 - suffix.length) + suffix;
        if (!taken.has(candidate)) {
            taken.add(candidate);
            return candidate;
        }
    }
    const fallback = `${name.slice(0, 26)}${taken.size}`;
    taken.add(fallback);
    return fallback;
}

// --- Reading the bank's own words -------------------------------------------
// Their Операция, Сумма and Остаток cells are the SMS lines themselves —
// `🔴 Platezh`, `➖ 1 000 000.00 UZS`, `💵 7 574 438.62 UZS`. Taking them
// verbatim is both more faithful and less to maintain than modelling each bank's
// emoji vocabulary and decimal separators.
const AMOUNT_MARKERS = ['➖', '➕'];
const BALANCE_MARKERS = ['💵', '💰'];
const MAX_OPERATION_LEN = 60;

function messageLines(raw: string | null | undefined): string[] {
    return String(raw || '').split('\n').map((line) => line.trim()).filter(Boolean);
}

/** Every marker that names a field rather than the operation. */
const FIELD_MARKERS = [
    ...AMOUNT_MARKERS, ...BALANCE_MARKERS, '📍', '🏪', '💳',
    '🕓', '🕘', '🕒', '📅',
];

/**
 * The first line, when it is one of the bank's headline lines.
 *
 * A headline opens with a symbol. A receipt that starts with `[Photo]`, a
 * transfer description or an OCR dump does not, and falls back to our own label.
 */
export function operationFromRaw(raw: string | null | undefined): string | null {
    const [first] = messageLines(raw);
    if (!first || first.length > MAX_OPERATION_LEN) return null;
    // `ℹ️` is U+2139 plus U+FE0F, and U+2139 on its own is a *letter* (it
    // lives among the letterlike symbols), so `\p{So}` alone drops the
    // balance-change notices. The selector is what makes it an emoji.
    if (!/^\p{S}|^.️/u.test(first)) return null;
    // `📍 PLUM OPLATA, UZ` opens with a symbol too; a receipt that arrives
    // without its headline must not have its merchant read as the operation.
    return FIELD_MARKERS.some((m) => first.startsWith(m)) ? null : first;
}

function lineStartingWith(raw: string | null | undefined, markers: string[]): string | null {
    return messageLines(raw).find((line) => markers.some((m) => line.startsWith(m))) ?? null;
}

/** `➖ 5 000.00 UZS`, exactly as the bank sent it. */
export function amountTextFromRaw(raw: string | null | undefined): string | null {
    return lineStartingWith(raw, AMOUNT_MARKERS);
}

/** `💵 4 229 872.82 UZS` — or `💰 …` on the banks that write it that way. */
export function balanceTextFromRaw(raw: string | null | undefined): string | null {
    return lineStartingWith(raw, BALANCE_MARKERS);
}

/**
 * Their number columns hold an unsigned whole number: `4 229 872.82` is
 * `4229872`. The sign is carried by the column beside it, which is why their own
 * total is a sum of magnitudes rather than a net.
 */
export function wholeAmount(value: number | null): number | null {
    if (value === null || Number.isNaN(value)) return null;
    return Math.trunc(Math.abs(value));
}

/**
 * Per-source presentation, for the receipts that carry no headline. CardXabar
 * tags every operation with one red dot and writes `1 000.00`; HUMO Card uses a
 * per-type emoji, the European `1.000,00` and `💰` for the balance.
 */
export type SourceStyle = {
    uniformEmoji: string | null;
    thousands: string;
    decimal: string;
    balanceEmoji: string;
};

const STYLE_CARDXABAR: SourceStyle = { uniformEmoji: '🔴', thousands: ' ', decimal: '.', balanceEmoji: '💵' };
const STYLE_HUMO: SourceStyle = { uniformEmoji: null, thousands: '.', decimal: ',', balanceEmoji: '💰' };

export function styleForSource(sourceDisplay: string): SourceStyle {
    return (sourceDisplay || '').toUpperCase().includes('HUMO') ? STYLE_HUMO : STYLE_CARDXABAR;
}

const TYPE_EMOJI: Record<string, string> = { DEBIT: '💸', CREDIT: '🎉', CONVERSION: '💱', REVERSAL: '↩️' };
const WITHDRAWAL_HINTS = ['снятие', 'обнал', 'bxo', 'atm', 'банкомат'];
/** A balance-change notice is not a payment; their sheet marks it `ℹ️`. */
const NOTICE_HINTS = ['изменен', 'изменение', 'счет по карте'];

function groupDigits(value: number, style: SourceStyle): string {
    const [whole, frac] = Math.abs(value).toFixed(2).split('.');
    let grouped = '';
    for (let offset = 0; offset < whole.length; offset += 1) {
        if (offset && offset % 3 === 0) grouped = style.thousands + grouped;
        grouped = whole[whole.length - 1 - offset] + grouped;
    }
    return `${grouped}${style.decimal}${frac}`;
}

/** `➖ 5 000.00 UZS` — their decorated amount, in the source's own convention. */
export function formatMoney(value: number | null, currency: string, style: SourceStyle): string {
    if (value === null || Number.isNaN(value)) return '';
    return `${value < 0 ? '➖' : '➕'} ${groupDigits(value, style)} ${currency || 'UZS'}`;
}

/** `💵 4 229 872.82 UZS` — the balance is never signed in their sheet. */
export function formatBalance(value: number | null, currency: string, style: SourceStyle): string {
    if (value === null || Number.isNaN(value)) return '';
    return `${style.balanceEmoji} ${groupDigits(value, style)} ${currency || 'UZS'}`;
}

/** The Операция cell for receipts that arrived without a headline line. */
export function clientOperationLabel(row: Transaction, fallback: string, style: SourceStyle): string {
    if (!fallback) return '';
    // Order matters and follows the Python: a source that tags everything with
    // one emoji keeps it, notice or not — their CardXabar column holds only
    // 🔴 and 🟢 across all 1011 rows.
    if (style.uniformEmoji) return `${style.uniformEmoji} ${fallback}`;
    if (NOTICE_HINTS.some((hint) => fallback.toLowerCase().includes(hint))) return `ℹ️ ${fallback}`;
    const haystack = `${row.operator_raw || ''} ${row.raw_message || ''}`.toLowerCase();
    if (row.transaction_type === 'DEBIT' && WITHDRAWAL_HINTS.some((hint) => haystack.includes(hint))) {
        return `🏧 ${fallback}`;
    }
    return `${TYPE_EMOJI[row.transaction_type] || 'ℹ️'} ${fallback}`;
}

/** Their sign column: `-` for money out, `+` for money in. */
export function directionSign(transactionType: string): string {
    if (transactionType === 'CREDIT' || transactionType === 'REVERSAL') return '+';
    if (transactionType === 'DEBIT' || transactionType === 'CONVERSION') return '-';
    return '';
}

/**
 * HUMO's hidden helpers: `40000` becomes ` 40.000`, its length, and the thousand
 * groups spread over `мрд`/`млн`/`тыс`/`ед`. Their formula leaves the leading
 * group a number and the rest text, so a `000` group keeps its zeros.
 */
export function digitGroups(value: number | null, prefix: string, leadingSpace: boolean): RowValues {
    const names = ['units', 'thousands', 'millions', 'billions'];
    const out: RowValues = { [`${prefix}_digits`]: 0, [`${prefix}_length`]: 0 };
    for (const name of names) out[`${prefix}_${name}`] = 0;
    if (!value) return out;
    const grouped = String(Math.trunc(value)).replace(/\B(?=(\d{3})+(?!\d))/g, '.');
    const text = (leadingSpace ? ' ' : '') + grouped;
    out[`${prefix}_digits`] = text;
    out[`${prefix}_length`] = text.length;
    const groups = grouped.split('.').reverse();
    groups.forEach((group, position) => {
        out[`${prefix}_${names[position]}`] = position === groups.length - 1 ? parseInt(group, 10) : group;
    });
    return out;
}

/**
 * A Date whose UTC parts equal the wall-clock parts of a naive timestamp.
 *
 * `transaction_date` has no timezone — it is local wall time. ExcelJS serialises
 * a Date by its UTC components, so handing it a normally-parsed value shifts
 * every cell by the machine's offset. Excel has no notion of timezone in a date
 * cell, so the honest representation is the clock reading the bank printed.
 */
export function excelDate(raw: unknown): Date | null {
    if (!raw) return null;
    const m = String(raw).match(/^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2}))?)?/);
    if (!m) return null;
    const [, y, mo, d, hh = '0', mi = '0', ss = '0'] = m;
    return new Date(Date.UTC(+y, +mo - 1, +d, +hh, +mi, +ss));
}

/** A time of day on Excel's day zero, which is how their `Время` column reads it. */
function excelTime(stamp: Date): Date {
    return new Date(Date.UTC(1899, 11, 30, stamp.getUTCHours(), stamp.getUTCMinutes(), stamp.getUTCSeconds()));
}

/** Render one transaction into the values of the customer's columns. */
export function buildRowValues(row: Transaction, articleLabels?: Record<string, string>): RowValues {
    const style = styleForSource(row.source_display || row.source_channel || '');
    const num = (raw: unknown): number | null => {
        if (raw === null || raw === undefined || raw === '') return null;
        const parsed = parseFloat(String(raw));
        return Number.isNaN(parsed) ? null : parsed;
    };
    const amount = num(row.amount);
    const balance = num(row.balance_after);
    const currency = row.currency || 'UZS';
    const labels: Record<string, string> = { DEBIT: 'Списание', CREDIT: 'Пополнение', CONVERSION: 'Конверсия', REVERSAL: 'Отмена' };
    const fallbackLabel = row.transaction_type_display || labels[row.transaction_type] || '';

    const code = row.annotation_article_code?.trim() || '';
    const label = code ? articleLabels?.[code] : undefined;

    return {
        date_time: excelDate(row.transaction_date),
        source: row.source_display || row.source_channel || '',
        operation: operationFromRaw(row.raw_message) || clientOperationLabel(row, fallbackLabel, style),
        operator: row.operator_raw || '',
        application: row.application_mapped || 0,
        sign: directionSign(row.transaction_type),
        amount_text: amountTextFromRaw(row.raw_message) || formatMoney(amount, currency, style),
        amount_value: wholeAmount(amount),
        balance_text: balanceTextFromRaw(row.raw_message) || formatBalance(balance, currency, style) || 0,
        balance_value: balance === null ? 0 : wholeAmount(balance),
        card: row.card_last_4 || '',
        p2p: row.is_p2p ? 1 : 0,
        p2p_confirmed: row.is_p2p ? 1 : null,
        details: row.annotation_details || '',
        person: row.annotation_person_code || '',
        period: row.annotation_period ? excelDate(`${String(row.annotation_period).slice(0, 10)}T00:00:00`) : null,
        article: label ? `${code}=${label}` : code,
        raw_message: row.raw_message || '',
    };
}

/**
 * The marker each import column is filled from.
 *
 * The banks do not agree on line order — CardXabar sends the card before the
 * merchant, HUMO the other way, NBU varies between its own messages — so the
 * column is chosen by what the line starts with, the way their formulas pick it
 * out, rather than by where it appeared.
 */
const IMPORT_ROLE_MARKERS: Array<[string, string[]]> = [
    ['amount', ['➖', '➕']],
    ['operator', ['📍', '🏪']],
    ['card', ['💳']],
    ['balance', ['💵', '💰']],
];

/**
 * One receipt's lines sorted into the roles their import columns name. A line
 * the markers do not claim — an OCR dump has no `📍` — fills the unclaimed roles
 * left to right, so nothing is silently dropped.
 */
function importRoles(lines: string[], when: Date | null, order: string[]): Record<string, unknown> {
    const roles: Record<string, unknown> = { headline: lines[0] ?? 0, when: when ?? 0 };
    const claimed = new Set<number>(lines.length ? [0] : []);
    for (const [role, markers] of IMPORT_ROLE_MARKERS) {
        const index = lines.findIndex((line, i) => !claimed.has(i) && markers.some((m) => line.startsWith(m)));
        if (index >= 0) {
            roles[role] = lines[index];
            claimed.add(index);
        }
    }
    if (claimed.size > (lines.length ? 1 : 0)) {
        // A marked receipt: a column with no marker of its own stays empty. The
        // clock line belongs to no column, so filling from the leftovers would
        // put it under `Сумма остатка`.
        for (const role of order) if (!(role in roles)) roles[role] = 0;
        return roles;
    }
    // Nothing was marked — an OCR dump or a photo caption. Its lines fill the
    // columns left to right so that nothing is dropped.
    const spare = lines.filter((_line, i) => !claimed.has(i));
    let next = 0;
    for (const role of order) {
        if (!(role in roles)) roles[role] = spare[next++] ?? 0;
    }
    return roles;
}

/**
 * What their import group holds for one receipt: the Telegram export line by
 * line — the `CardXabar, [01.01.2026 7:17]` header, the message, a blank — with
 * the fields their formulas pull out of it beside the header line.
 */
export function importLines(
    values: RowValues,
    messageNo: number,
    template: SheetTemplate = CARDXABAR,
): RowValues[] {
    const when = values.date_time instanceof Date ? values.date_time : null;
    const source = String(values.source || '');
    const lines = messageLines(String(values.raw_message || '')).slice(0, IMPORT_LINES_PER_MESSAGE);
    const pad = (n: number) => String(n).padStart(2, '0');
    const stamp = when
        ? `${pad(when.getUTCDate())}.${pad(when.getUTCMonth() + 1)}.${when.getUTCFullYear()} ${when.getUTCHours()}:${pad(when.getUTCMinutes())}`
        : '';
    const weekday = when ? ((when.getUTCDay() + 6) % 7) + 1 : 0;
    const header: RowValues = {
        line: `${source}, [${stamp}]`,
        message_no: messageNo,
        header_no: messageNo,
        service: source,
        date: when ? new Date(Date.UTC(when.getUTCFullYear(), when.getUTCMonth(), when.getUTCDate())) : 0,
        time: when ? excelTime(when) : 0,
        weekday,
    };
    const roles = importRoles(lines, when, template.importOrder);
    template.importOrder.forEach((role, index) => {
        header[`line_${index + 1}`] = roles[role] ?? 0;
    });
    const rest: RowValues = {};
    for (const key of Object.values(IMPORT_COLUMNS)) rest[key] = 0;
    rest.message_no = messageNo;
    return [header, ...lines.map((line) => ({ ...rest, line })), { ...rest, line: null }];
}

// --- Writing ------------------------------------------------------------------
function decodeTemplate(): Uint8Array {
    const binary = atob(EXCEL_CLIENT_TEMPLATE_BASE64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
    return bytes;
}

type ConditionalFormattingEntry = { ref: string; rules: Array<{ type: string } | undefined> };

/** The conditional-format rule types ExcelJS can write back out. */
const RENDERABLE_RULES = new Set([
    'expression', 'cellIs', 'top10', 'aboveAverage', 'dataBar', 'colorScale', 'iconSet', 'containsText', 'timePeriod',
]);

async function loadTemplate(Excel: ExcelModule): Promise<ExcelJS.Workbook> {
    const workbook = new Excel.Workbook();
    // ExcelJS types the argument as a Node Buffer; its browser build takes an ArrayBuffer.
    await workbook.xlsx.load(decodeTemplate().buffer as never);
    // A rule ExcelJS cannot render (their `duplicateValues` tint on HUMO) would be
    // written as an empty <conditionalFormatting>, which Excel refuses to open.
    for (const sheet of workbook.worksheets) {
        // Not in the typings, but it is where a loaded sheet keeps its rules.
        const holder = sheet as unknown as { conditionalFormattings: ConditionalFormattingEntry[] };
        holder.conditionalFormattings = (holder.conditionalFormattings || [])
            .map((cf) => ({ ...cf, rules: (cf.rules || []).filter((rule) => rule && RENDERABLE_RULES.has(rule.type)) }))
            .filter((cf) => cf.rules.length > 0);
    }
    return workbook;
}

const EXCEL_DAY_ZERO = Date.UTC(1899, 11, 30);

/** What their view would hide: a zero, in any of its dressings. */
function isHiddenZero(value: unknown): boolean {
    if (value === 0) return true;
    return value instanceof Date && value.getTime() === EXCEL_DAY_ZERO;
}

function toCellValue(value: unknown): CellValue {
    if (value === null || value === undefined || value === '') return null;
    if (isHiddenZero(value)) return null;
    if (value instanceof Date || typeof value === 'number' || typeof value === 'string') return value;
    return String(value);
}

export type ClientWorkbookOptions = {
    /** Newest-first exports keep `1` on the oldest row of each sheet. */
    descending?: boolean;
    /** code -> label, so the article column can render `р1=ФЗП,ФМП,МП`. */
    articleLabels?: Record<string, string>;
    /** Whether to fill their hidden import group with the receipt text. */
    withImportBlock?: boolean;
};

/** Who a sheet is for: a bank named in its captions, or the combined sheet's own wording. */
type SheetNaming = { source?: string; captions?: Record<string, string> };

/**
 * The customer's workbook: every receipt on the first sheet, then one sheet per
 * source in their order, each laid out by the template sheet for that bank.
 */
export async function buildClientWorkbook(
    Excel: ExcelModule,
    rows: Transaction[],
    options: ClientWorkbookOptions = {},
): Promise<ExcelJS.Workbook> {
    // The output starts as the template itself. Its own two sheets go, and every
    // sheet of the export is a fresh copy of the template sheet for its bank,
    // added in the order the workbook should have.
    const workbook = await loadTemplate(Excel);
    for (const template of SHEET_TEMPLATES) {
        const original = workbook.getWorksheet(template.sheet);
        if (original) workbook.removeWorksheet(original.id);
    }
    const grouped = groupBySource(rows);
    const taken = new Set<string>();

    const combined = await cloneTemplateSheet(Excel, workbook, CARDXABAR.sheet, sanitizeSheetName(COMBINED_SHEET_NAME, taken));
    fillSheet(combined, CARDXABAR, rows, 1, options, { captions: COMBINED_CAPTIONS });

    // The second numbering column continues across the per-bank sheets, the way
    // their HUMO sheet opens at 1012; the combined sheet above counts on its own.
    let operationNumber = 0;
    for (const source of orderSources(grouped.keys())) {
        const template = templateForSource(source);
        const sheet = await cloneTemplateSheet(Excel, workbook, template.sheet, sanitizeSheetName(source, taken));
        const sheetRows = grouped.get(source) || [];
        fillSheet(sheet, template, sheetRows, operationNumber + 1, options, { source });
        operationNumber += sheetRows.length;
    }

    // Open on the first sheet, and on it alone: the template's HUMO view carries
    // a selected tab, and two selected tabs would open the sheets grouped.
    workbook.views = [{ x: 0, y: 0, width: 20490, height: 7620, firstSheet: 0, activeTab: 0, visibility: 'visible' }];
    workbook.worksheets.forEach((sheet, index) => {
        sheet.views = sheet.views.map((view) => ({ ...view, tabSelected: index === 0 } as unknown as ExcelJS.WorksheetView));
    });
    return workbook;
}

/**
 * A copy of a template sheet under its own name.
 *
 * ExcelJS has no sheet copy; assigning one sheet's model to a new sheet is the
 * accepted way. The model comes from a freshly loaded template so no row objects
 * are shared between sheets.
 */
async function cloneTemplateSheet(
    Excel: ExcelModule,
    workbook: ExcelJS.Workbook,
    sheetName: string,
    name: string,
): Promise<ExcelJS.Worksheet> {
    const fresh = await loadTemplate(Excel);
    const source = fresh.getWorksheet(sheetName) as ExcelJS.Worksheet;
    const target = workbook.addWorksheet(name);
    const sourceModel = (source as unknown as { model: Record<string, unknown> }).model;
    // The model reports merges under one key and reads them back under another.
    const model = { ...sourceModel, name, id: target.id, mergeCells: sourceModel.merges };
    (target as unknown as { model: Record<string, unknown> }).model = model;
    return target;
}

function fillSheet(
    sheet: ExcelJS.Worksheet,
    template: SheetTemplate,
    rows: Transaction[],
    firstOperationNumber: number,
    options: ClientWorkbookOptions,
    naming: SheetNaming,
): void {
    const columnCount = sheet.columnCount;
    const values = rows.map((row) => buildRowValues(row, options.articleLabels));

    // The counts their header rows carry: how many, the latest, the gross total.
    const stamps = values.map((v) => v.date_time).filter((d): d is Date => d instanceof Date);
    const figures: RowValues = {
        count: rows.length,
        last_operation_at: stamps.length ? new Date(Math.max(...stamps.map((d) => d.getTime()))) : null,
        total_amount: rows.length ? values.reduce((sum, v) => sum + (Number(v.amount_value) || 0), 0) : null,
    };
    for (const [ref, key] of Object.entries(template.figures)) {
        sheet.getCell(ref).value = toCellValue(figures[key]) as ExcelJS.CellValue;
    }
    const source = (naming.source || '').trim();
    if (naming.captions) {
        for (const [ref, text] of Object.entries(naming.captions)) sheet.getCell(ref).value = text;
    } else if (source && !isNativeSource(template, source)) {
        for (const ref of template.captions) {
            const cell = sheet.getCell(ref);
            if (typeof cell.value === 'string') cell.value = cell.value.split(template.token).join(source);
        }
    }
    // Their row of zeros above the data would show without the view that hides zeros.
    for (let r = 1; r <= template.headerRows; r += 1) {
        sheet.getRow(r).eachCell({ includeEmpty: false }, (cell) => {
            if (isHiddenZero(cell.value)) cell.value = null;
        });
    }

    const keys: (string | null)[] = [];
    const firstStyles: ExcelJS.Style[] = [];
    const bodyStyles: ExcelJS.Style[] = [];
    for (let c = 1; c <= columnCount; c += 1) {
        const letter = sheet.getColumn(c).letter;
        keys.push(IMPORT_COLUMNS[letter] || template.columns[letter] || null);
        firstStyles.push({ ...sheet.getRow(template.firstRow).getCell(c).style } as ExcelJS.Style);
        bodyStyles.push({ ...sheet.getRow(template.bodyRow).getCell(c).style } as ExcelJS.Style);
    }

    const wantsDigits = Object.values(template.columns).includes('amount_digits');
    const start = firstOperationNumber;
    const numbers = rows.map((_, i) => (options.descending ? start + rows.length - 1 - i : start + i));
    const withImportBlock = options.withImportBlock ?? true;
    const lines: RowValues[] = [];
    if (withImportBlock) {
        values.forEach((v, i) => { lines.push(...importLines(v, i + 1, template)); });
    }

    const seen = new Map<number, number>();
    let rowIndex = template.firstRow;
    const writeRow = (data: RowValues, styles: ExcelJS.Style[], onlyImport: boolean) => {
        const row = sheet.getRow(rowIndex);
        for (let c = 1; c <= columnCount; c += 1) {
            const cell = row.getCell(c);
            if (onlyImport && c > IMPORT_LAST_COLUMN) {
                cell.value = null;
                cell.style = {} as ExcelJS.Style;
                continue;
            }
            const key = keys[c - 1];
            cell.value = toCellValue(key ? data[key] : null) as ExcelJS.CellValue;
            cell.style = { ...styles[c - 1] };
        }
        rowIndex += 1;
    };

    values.forEach((base, i) => {
        const data: RowValues = { ...base, row_number: i + 1, operation_number: numbers[i] };
        const stamp = data.date_time instanceof Date ? data.date_time : null;
        if (stamp) {
            const repeat = seen.get(stamp.getTime()) || 0;
            seen.set(stamp.getTime(), repeat + 1);
            data.date_time_dedup = repeat ? new Date(stamp.getTime() + repeat * DEDUP_STEP_MS) : stamp;
        }
        if (wantsDigits) {
            Object.assign(data, digitGroups(Number(data.amount_value) || 0, 'amount', true));
            Object.assign(data, digitGroups(Number(data.balance_value) || 0, 'balance', false));
        }
        if (lines[i]) Object.assign(data, lines[i]);
        writeRow(data, i === 0 ? firstStyles : bodyStyles, false);
    });
    for (let i = values.length; i < lines.length && rowIndex <= EXCEL_MAX_ROWS; i += 1) {
        writeRow(lines[i], bodyStyles, true);
    }
    // The template's own sample rows, where nothing was written over them.
    for (let r = rowIndex; r <= template.bodyRow; r += 1) {
        sheet.getRow(r).eachCell({ includeEmpty: true }, (cell) => {
            cell.value = null;
            cell.style = {} as ExcelJS.Style;
        });
    }
}
