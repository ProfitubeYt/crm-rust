/* Администрирование CRM. Без сборки и зависимостей.

   Страница сама ничего не решает и ничего не хранит. Токен доступа живёт в
   памяти вкладки; сессию продлевает HttpOnly-cookie, которую скрипт прочитать
   не может. Всё, что здесь показано и нажато, сервер проверяет заново на
   каждом запросе: скрытая кнопка защитой не является. */
(() => {
  'use strict';

  // ------------------------------------------------------------------ DOM

  const $ = (id) => document.getElementById(id);

  function el(tag, props, ...children) {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(props || {})) {
      if (value === undefined || value === null) continue;
      if (key === 'class') node.className = value;
      else if (key === 'text') node.textContent = value;
      else if (key === 'on') for (const [name, handler] of Object.entries(value)) node.addEventListener(name, handler);
      else if (key === 'attrs') for (const [name, text] of Object.entries(value)) node.setAttribute(name, text);
      else node[key] = value;
    }
    for (const child of children.flat(Infinity)) {
      if (child === null || child === undefined || child === false) continue;
      node.append(child instanceof Node ? child : document.createTextNode(String(child)));
    }
    return node;
  }

  const chip = (text, kind) => el('span', { class: kind ? `chip ${kind}` : 'chip', text });
  const field = (label, control) => el('label', { class: 'field' }, label, control);
  const section = (title, ...children) => el('div', { class: 'section' }, el('h4', { text: title }), children);

  function toast(text, failed) {
    const node = el('div', { class: failed ? 'toast err' : 'toast', text });
    $('toasts').append(node);
    setTimeout(() => node.remove(), failed ? 8000 : 4000);
  }

  function confirmAction(title, text, yes) {
    return new Promise((resolve) => {
      $('confirm-title').textContent = title;
      $('confirm-text').textContent = text;
      $('confirm-yes').textContent = yes || 'Подтвердить';
      $('confirm').hidden = false;
      $('confirm-no').focus();
      const done = (answer) => {
        $('confirm').hidden = true;
        $('confirm-yes').onclick = null;
        $('confirm-no').onclick = null;
        resolve(answer);
      };
      $('confirm-yes').onclick = () => done(true);
      $('confirm-no').onclick = () => done(false);
    });
  }

  // ------------------------------------------------------------------ тексты

  const TEXT = {
    network: 'Нет связи с сервером.',
    invalid_credentials: 'Неверный логин или пароль.',
    locked: 'Вход временно заблокирован после неудачных попыток.',
    inactive_user: 'Учётная запись отключена.',
    rate_limited: 'Слишком много попыток. Повторите немного позже.',
    invalid_code: 'Неверный код.',
    invalid_temp_token: 'Время на ввод кода истекло. Войдите заново.',
    requires_2fa_setup:
      'Для этой учётной записи нужно сначала настроить двухфакторную аутентификацию. '
      + 'Это делается при входе в приложение TBS; после настройки вернитесь сюда.',
    global_admin_required:
      'У этой учётной записи нет прав администратора CRM. Их выдаёт другой администратор CRM.',
    access_expired: 'Срок доступа этой учётной записи истёк.',
    last_admin: 'Нельзя: это последний действующий администратор CRM.',
    cannot_disable_self: 'Собственную учётную запись отключить нельзя — это делает другой администратор.',
    cannot_demote_self: 'Снять права администратора CRM с себя нельзя — это делает другой администратор.',
    username_exists: 'Такой логин уже занят.',
    invalid_username: 'Логин пустой или длиннее 50 символов.',
    weak_password: 'Пароль слишком простой: проверьте требования под полем.',
    invalid_password: 'Пароль не задан.',
    telegram_id_already_linked: 'Этот Telegram ID уже привязан к другой учётной записи.',
    invalid_workspace_role: 'В этом рабочем пространстве нет такой роли.',
    unknown_workspace: 'Сервер не знает такого рабочего пространства. Обновите страницу.',
    invalid_workspaces: 'Сервер не принял список рабочих пространств.',
    user_not_found: 'Учётная запись не найдена. Обновите страницу.',
    session_store_unavailable: 'Хранилище сессий сейчас недоступно. Повторите позже.',
    system_access_denied: 'Сервер отклонил запрос на уровне системного доступа. Нужен владелец сервера.',
    system_access_blocked: 'Сервер заблокирован: не найдена его конфигурация доступа. Нужен владелец сервера.',
    csrf_denied: 'Сервер отклонил запрос с этого адреса.',
    validation: 'Сервер не принял данные: проверьте заполненные поля.',
  };

  const FIELDS = {
    display_name: 'имя', role: 'роль в Чеках', is_active: 'состояние', allowed_tabs: 'вкладки',
    allowed_folders: 'папки', allowed_sources: 'источники', forbidden_periods: 'закрытые периоды',
    can_toggle_sources: 'переключение источников', force_2fa: 'требование 2FA',
    session_ttl_days: 'срок сессии', access_until: 'срок доступа', access_note: 'примечание',
    global_admin: 'администратор CRM', workspaces: 'рабочие пространства',
  };

  const ACTIONS = {
    login_success: 'Вход',
    login_failed: 'Неудачный вход',
    admin_user_create: 'Создана учётная запись',
    admin_user_update: 'Изменена учётная запись',
    admin_user_access_change: 'Изменён доступ',
    admin_user_reset_password: 'Задан новый пароль',
    admin_user_reset_2fa: 'Сброшена 2FA',
    admin_user_force_2fa: 'Требование 2FA',
    admin_user_unlock: 'Снята блокировка входа',
    admin_user_deactivate: 'Учётная запись отключена',
    admin_user_link_telegram: 'Привязан Telegram',
    admin_user_unlink_telegram: 'Отвязан Telegram',
    admin_user_revoke_sessions: 'Завершены сессии',
    admin_kill_session: 'Завершена сессия',
    admin_kill_all_sessions: 'Завершены все сессии, кроме текущей',
    admin_bootstrap_create: 'Администратор создан с сервера',
    admin_bootstrap_promote: 'Администратор назначен с сервера',
  };

  // ------------------------------------------------------------------ время

  const dateTime = new Intl.DateTimeFormat('ru-RU', { dateStyle: 'short', timeStyle: 'short' });
  const dateOnly = new Intl.DateTimeFormat('ru-RU', { dateStyle: 'long' });

  function parseTime(value) {
    if (value === null || value === undefined || value === '') return null;
    let text = String(value);
    if (/^\d+(\.\d+)?$/.test(text)) return new Date(Number(text) * 1000);
    // Сервер пишет время в UTC; запись без пояса — тоже UTC.
    if (/^\d{4}-\d{2}-\d{2}T[\d:.]+$/.test(text)) text += 'Z';
    const date = new Date(text);
    return Number.isNaN(date.getTime()) ? null : date;
  }

  const when = (value) => { const date = parseTime(value); return date ? dateTime.format(date) : '—'; };
  const day = (value) => { const date = parseTime(value); return date ? dateOnly.format(date) : ''; };
  // Дата окончания доступа как её видит поле ввода: YYYY-MM-DD.
  const dayValue = (value) => (value ? String(value).slice(0, 10) : '');

  // ------------------------------------------------------------------ API

  const LAUNCH_KEY = 'crm.launch';
  const session = { token: null, launch: sessionStorage.getItem(LAUNCH_KEY) };

  class ApiError extends Error {
    constructor(status, code, payload) {
      super(code || String(status));
      this.status = status;
      this.code = code || '';
      this.payload = payload || null;
      this.handled = false;
    }
  }

  function codeOf(payload) {
    if (!payload || typeof payload !== 'object') return '';
    const detail = payload.detail;
    if (Array.isArray(detail)) return 'validation';
    if (detail && typeof detail === 'object') return String(detail.error || payload.error_code || '');
    if (typeof detail === 'string') return detail;
    return String(payload.error || payload.error_code || '');
  }

  function detailOf(error) {
    const payload = error && error.payload;
    if (!payload) return {};
    return payload.detail && typeof payload.detail === 'object' && !Array.isArray(payload.detail)
      ? payload.detail
      : payload;
  }

  async function request(method, path, body, anonymous) {
    const headers = { Accept: 'application/json' };
    if (body !== undefined) headers['Content-Type'] = 'application/json';
    if (session.token && !anonymous) headers.Authorization = `Bearer ${session.token}`;
    if (session.launch) headers['X-Launch-Session'] = session.launch;
    let response;
    try {
      response = await fetch(path, {
        method,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
        credentials: 'same-origin',
        cache: 'no-store',
      });
    } catch (_) {
      throw new ApiError(0, 'network');
    }
    const text = await response.text();
    let payload = null;
    if (text) {
      try { payload = JSON.parse(text); } catch (_) { payload = null; }
    }
    if (response.ok) return payload;
    throw new ApiError(response.status, codeOf(payload), payload);
  }

  const isLaunch = (error) => error.code === 'launch_required' || error.code === 'launch_expired';

  let renewing = null;
  // Новый токен доступа по cookie продления. Один запрос на всех, кто ждёт.
  function renew() {
    if (!renewing) {
      renewing = request('POST', '/api/auth/refresh', {}, true)
        .then((answer) => {
          session.token = answer && answer.ok && answer.token ? answer.token : null;
          return { ok: Boolean(session.token), launch: false };
        })
        .catch((error) => ({ ok: false, launch: isLaunch(error) }))
        .finally(() => { renewing = null; });
    }
    return renewing;
  }

  async function api(method, path, body) {
    try {
      return await request(method, path, body);
    } catch (error) {
      if (error.status === 401) {
        const renewed = await renew();
        if (renewed.ok) return request(method, path, body);
        error.handled = true;
        leave('Сессия завершена. Войдите снова.');
      } else if (isLaunch(error)) {
        error.handled = true;
        forgetLaunch();
        leave('');
        askLaunch(resume);
      } else if (error.code === 'global_admin_required') {
        error.handled = true;
        leave(TEXT.global_admin_required);
      }
      throw error;
    }
  }

  function messageOf(error) {
    if (!(error instanceof ApiError)) return 'Не удалось выполнить действие.';
    if (TEXT[error.code]) return TEXT[error.code];
    if (error.status === 422) return TEXT.validation;
    if (error.status === 429) return TEXT.rate_limited;
    if (error.status >= 500) return `Сервер ответил ошибкой (${error.status}). Повторите позже.`;
    return `Сервер отказал: ${error.code || error.status}.`;
  }

  function report(error) {
    if (error && error.handled) return;
    if (!(error instanceof ApiError)) console.error(error);
    toast(messageOf(error), true);
  }

  // ------------------------------------------------------------------ вход

  const state = { meta: null, users: [], view: 'users', sheet: null };
  const login = { step: 'password', temp: null, after: null };

  function setLoginError(text) {
    $('login-error').textContent = text || '';
    $('login-error').hidden = !text;
  }

  function setStep(step) {
    login.step = step;
    $('login-step-password').hidden = step !== 'password';
    $('login-step-code').hidden = step !== 'code';
    $('login-step-launch').hidden = step !== 'launch';
    $('login-back').hidden = step === 'password';
    $('login-submit').textContent = step === 'password' ? 'Войти' : 'Продолжить';
    const focus = { password: 'login-username', code: 'login-code', launch: 'login-launch' }[step];
    $(focus).focus();
  }

  function showLogin(message) {
    $('view-app').hidden = true;
    $('view-login').hidden = false;
    $('login-password').value = '';
    $('login-code').value = '';
    login.temp = null;
    setStep('password');
    setLoginError(message);
  }

  function askLaunch(after) {
    login.after = after;
    $('view-app').hidden = true;
    $('view-login').hidden = false;
    $('login-launch').value = '';
    setStep('launch');
    setLoginError('');
  }

  function forgetLaunch() {
    session.launch = null;
    sessionStorage.removeItem(LAUNCH_KEY);
  }

  function loginMessage(error) {
    const detail = detailOf(error);
    let text = messageOf(error);
    if (error.code === 'invalid_credentials' && Number.isFinite(detail.attempts_left)) {
      text += ` Осталось попыток: ${detail.attempts_left}.`;
    }
    if (error.code === 'locked' && detail.locked_seconds) {
      text += ` Повторите через ${Math.ceil(detail.locked_seconds / 60)} мин.`;
    }
    if (error.code === 'rate_limited' && detail.retry_after_seconds) {
      text += ` Через ${detail.retry_after_seconds} с.`;
    }
    return text;
  }

  async function sendPassword() {
    const username = $('login-username').value.trim();
    const password = $('login-password').value;
    if (!username || !password) {
      setLoginError('Введите логин и пароль.');
      return;
    }
    const answer = await request('POST', '/api/auth/login', { username, password }, true);
    $('login-password').value = '';
    if (answer && answer.requires_2fa) {
      if (answer.requires_2fa_setup) {
        setLoginError(TEXT.requires_2fa_setup);
        return;
      }
      login.temp = answer.temp_token;
      $('login-code').value = '';
      setStep('code');
      return;
    }
    if (!answer || !answer.ok || !answer.token) {
      throw new ApiError(401, (answer && answer.error) || 'invalid_credentials', answer);
    }
    session.token = answer.token;
    await enter();
  }

  async function sendCode() {
    const code = $('login-code').value.replace(/\s+/g, '');
    if (code.length < 4) {
      setLoginError('Введите код.');
      return;
    }
    let answer;
    try {
      answer = await request('POST', '/api/auth/login/2fa', { temp_token: login.temp, code }, true);
    } catch (error) {
      if (error.code === 'invalid_temp_token' || error.code === 'permissions_outdated') {
        showLogin(TEXT.invalid_temp_token);
        return;
      }
      throw error;
    }
    if (!answer || !answer.ok || !answer.token) throw new ApiError(401, 'invalid_code', answer);
    session.token = answer.token;
    login.temp = null;
    await enter();
  }

  async function verifyLaunch() {
    const password = $('login-launch').value;
    if (!password) {
      setLoginError('Введите пароль запуска.');
      return;
    }
    const answer = await request('POST', '/api/security/app/verify-launch', { password }, true);
    $('login-launch').value = '';
    if (!answer || !answer.verified || !answer.session_token) {
      if (answer && answer.locked) {
        setLoginError(`Пароль запуска заблокирован до ${when(answer.locked_until)}.`);
      } else {
        const left = answer && Number.isFinite(answer.attempts_left) ? ` Осталось попыток: ${answer.attempts_left}.` : '';
        setLoginError(`Неверный пароль запуска.${left}`);
      }
      return;
    }
    session.launch = answer.session_token;
    sessionStorage.setItem(LAUNCH_KEY, session.launch);
    const after = login.after || resume;
    login.after = null;
    await after();
  }

  async function submitLogin(event) {
    event.preventDefault();
    setLoginError('');
    $('login-submit').disabled = true;
    try {
      if (login.step === 'launch') await verifyLaunch();
      else if (login.step === 'code') await sendCode();
      else await sendPassword();
    } catch (error) {
      setLoginError(loginMessage(error));
    } finally {
      $('login-submit').disabled = false;
    }
  }

  // Сессия по cookie, если она есть; иначе форма входа.
  async function resume() {
    const renewed = await renew();
    if (renewed.ok) return enter();
    if (renewed.launch) {
      forgetLaunch();
      return askLaunch(resume);
    }
    return showLogin('');
  }

  async function dropSession() {
    try {
      await request('POST', '/api/auth/logout', {});
    } catch (_) {
      // Сессия могла уже закончиться; токен забываем в любом случае.
    }
    session.token = null;
  }

  function leave(message) {
    session.token = null;
    state.users = [];
    closeSheet(true);
    showLogin(message);
  }

  // Панель открывается только тому, кому сервер отдал её справочные данные.
  async function enter() {
    let meta;
    try {
      meta = await request('GET', '/api/admin/meta');
    } catch (error) {
      if (isLaunch(error)) {
        forgetLaunch();
        askLaunch(enter);
        return;
      }
      if (error.code === 'global_admin_required') {
        // Вошёл, но не администратор: сессия здесь не нужна, закрываем её.
        await dropSession();
        showLogin(TEXT.global_admin_required);
        return;
      }
      session.token = null;
      showLogin(messageOf(error));
      return;
    }
    state.meta = meta;
    $('who-name').textContent = meta.me.display_name || meta.me.username || '';
    fillAccessFilter();
    $('view-login').hidden = true;
    $('view-app').hidden = false;
    showView('users');
    await loadUsers();
  }

  // ------------------------------------------------------------------ справочники

  const workspaceOf = (id) => state.meta.workspaces.find((item) => item.id === id);
  function roleTitle(workspaceId, roleId) {
    const workspace = workspaceOf(workspaceId);
    const role = workspace && workspace.roles.find((item) => item.id === roleId);
    return role ? role.title : roleId;
  }
  const lower = (text) => (text ? text[0].toLowerCase() + text.slice(1) : text);

  function fillAccessFilter() {
    const select = $('filter-access');
    select.replaceChildren(el('option', { value: '', text: 'Любой доступ' }));
    for (const workspace of state.meta.workspaces) {
      select.append(el('option', { value: `ws:${workspace.id}`, text: workspace.title }));
    }
    select.append(el('option', { value: 'none', text: 'Без рабочих пространств' }));
    select.append(el('option', { value: 'admin', text: 'Администраторы CRM' }));
  }

  // ------------------------------------------------------------------ список

  function accessChips(user) {
    const chips = [];
    if (user.global_admin) chips.push(chip('Администратор CRM', 'ok'));
    const roles = user.workspace_roles || {};
    for (const workspace of state.meta.workspaces) {
      if (roles[workspace.id]) chips.push(chip(`${workspace.title}: ${lower(roleTitle(workspace.id, roles[workspace.id]))}`));
    }
    if (!Object.keys(roles).length) chips.push(chip('Нет рабочих пространств', 'none'));
    return chips;
  }

  function stateChip(user) {
    const effective = user.effective_access || {};
    if (effective.state === 'disabled') return chip('Отключена', 'off');
    if (effective.state === 'expired') return chip('Срок истёк', 'warn');
    if (effective.locked) return chip('Вход заблокирован', 'warn');
    return chip('Активна', 'ok');
  }

  function telegramText(user) {
    if (!user.telegram_id) return '—';
    return user.telegram_username ? `@${user.telegram_username.replace(/^@/, '')} · ${user.telegram_id}` : String(user.telegram_id);
  }

  function visibleUsers() {
    const needle = $('search').value.trim().toLowerCase();
    const wantedState = $('filter-state').value;
    const wantedAccess = $('filter-access').value;
    return state.users.filter((user) => {
      const roles = user.workspace_roles || {};
      if (needle) {
        const hay = [user.username, user.display_name, user.telegram_username, user.telegram_id];
        if (!hay.some((value) => value !== null && value !== undefined && String(value).toLowerCase().includes(needle))) return false;
      }
      if (wantedState === 'active' && !user.is_active) return false;
      if (wantedState === 'disabled' && user.is_active) return false;
      if (wantedAccess === 'admin' && !user.global_admin) return false;
      if (wantedAccess === 'none' && Object.keys(roles).length) return false;
      if (wantedAccess.startsWith('ws:') && !roles[wantedAccess.slice(3)]) return false;
      return true;
    });
  }

  function renderUsers() {
    const rows = visibleUsers();
    const body = $('users').tBodies[0];
    body.replaceChildren(...rows.map((user) => {
      const open = () => openUser(user.id);
      return el('tr', {
        class: user.is_active ? '' : 'off',
        tabIndex: 0,
        on: { click: open, keydown: (event) => { if (event.key === 'Enter') open(); } },
      },
      el('td', { class: 'mono', text: user.username }),
      el('td', { text: user.display_name || '' }),
      el('td', null, el('div', { class: 'chips' }, accessChips(user))),
      el('td', { text: telegramText(user) }),
      el('td', { text: when(user.last_login_at) }),
      el('td', null, stateChip(user)));
    }));
    $('users-empty').hidden = rows.length > 0;
    $('count').textContent = rows.length === state.users.length
      ? `Всего: ${state.users.length}`
      : `Показано: ${rows.length} из ${state.users.length}`;
  }

  const byUsername = (a, b) => a.username.localeCompare(b.username, 'ru');

  async function loadUsers() {
    try {
      state.users = (await api('GET', '/api/admin/users?include_inactive=true')).sort(byUsername);
    } catch (error) {
      report(error);
      return;
    }
    renderUsers();
    renderWorkspaces();
  }

  function applyUser(user) {
    const at = state.users.findIndex((item) => item.id === user.id);
    if (at >= 0) state.users[at] = user;
    else state.users.push(user);
    state.users.sort(byUsername);
    renderUsers();
    renderWorkspaces();
  }

  // ------------------------------------------------------------------ рабочие пространства

  function renderWorkspaces() {
    if (!state.meta) return;
    const active = state.users.filter((user) => user.is_active);
    const cards = [];

    const admins = active.filter((user) => user.global_admin);
    cards.push(el('div', { class: 'card' },
      el('h3', { text: 'Администрирование CRM' }),
      el('p', { class: 'muted', text: 'Учётные записи, доступ к рабочим пространствам, сессии и настройки безопасности системы. Не относится ни к одному рабочему пространству и не открывает ни одно из них.' }),
      el('table', { class: 'roles' }, el('tbody', null, el('tr', null,
        el('td', null, el('strong', { text: 'Администратор CRM' }), el('div', { class: 'muted', text: admins.map((user) => user.username).join(', ') || 'никого' })),
        el('td', { class: 'num', text: String(admins.length) }))))));

    for (const workspace of state.meta.workspaces) {
      const holders = active.filter((user) => (user.workspace_roles || {})[workspace.id]);
      cards.push(el('div', { class: 'card' },
        el('h3', { text: workspace.title }),
        el('div', { class: 'chips' },
          workspace.available ? chip('Подключено', 'ok') : chip('На сервере не подключено', 'warn'),
          chip(`Учётных записей с доступом: ${holders.length}`)),
        el('p', { class: 'muted', text: workspace.description }),
        el('table', { class: 'roles' }, el('tbody', null, workspace.roles.map((role) => el('tr', null,
          el('td', null, el('strong', { text: role.title }), el('div', { class: 'muted', text: role.hint })),
          el('td', { class: 'num', text: String(holders.filter((user) => user.workspace_roles[workspace.id] === role.id).length) })))))));
    }
    $('workspaces').replaceChildren(...cards);
  }

  // ------------------------------------------------------------------ журнал

  function accessWords(access) {
    if (!access) return '';
    const parts = [];
    if (access.global_admin) parts.push('администратор CRM');
    const roles = access.workspace_roles || {};
    for (const workspace of state.meta.workspaces) {
      if (roles[workspace.id]) parts.push(`${workspace.title}: ${lower(roleTitle(workspace.id, roles[workspace.id]))}`);
    }
    if (!Object.keys(roles).length) parts.push('без рабочих пространств');
    return parts.join('; ');
  }

  function accessDiff(before, after) {
    if (!before || !after) return '';
    const parts = [];
    if (before.is_active !== after.is_active) parts.push(after.is_active ? 'включена' : 'отключена');
    if (Boolean(before.global_admin) !== Boolean(after.global_admin)) {
      parts.push(after.global_admin ? 'назначен администратором CRM' : 'снят с администраторов CRM');
    }
    const was = before.workspace_roles || {};
    const now = after.workspace_roles || {};
    for (const workspace of state.meta.workspaces) {
      if (was[workspace.id] === now[workspace.id]) continue;
      const from = was[workspace.id] ? lower(roleTitle(workspace.id, was[workspace.id])) : 'нет доступа';
      const to = now[workspace.id] ? lower(roleTitle(workspace.id, now[workspace.id])) : 'нет доступа';
      parts.push(`${workspace.title}: ${from} → ${to}`);
    }
    return parts.join('; ');
  }

  function describe(item) {
    const details = item.details || {};
    const target = details.username || (details.user_id ? `№ ${details.user_id}` : '');
    const tail = (text) => (text ? `${target}: ${text}` : target);
    switch (item.action) {
      case 'login_failed': return tail(TEXT[details.error] || details.error || '');
      case 'admin_user_create': return tail(accessWords(details.access));
      case 'admin_user_access_change': return tail(accessDiff(details.before, details.after));
      case 'admin_user_update': return tail((details.fields || []).map((name) => FIELDS[name] || name).join(', '));
      case 'admin_user_force_2fa': return tail(details.force_2fa ? 'включено' : 'выключено');
      case 'admin_user_link_telegram': return tail(`ID ${details.telegram_id}`);
      case 'admin_user_unlink_telegram': return tail(details.old_telegram_id ? `был ID ${details.old_telegram_id}` : '');
      case 'admin_user_reset_password':
      case 'admin_user_deactivate':
      case 'admin_user_revoke_sessions':
        return tail(Number.isFinite(details.sessions_ended) ? `завершено сессий: ${details.sessions_ended}` : '');
      case 'admin_kill_all_sessions': return `завершено сессий: ${details.killed_count}`;
      default: return target;
    }
  }

  function auditRows(items) {
    return items.map((item) => el('tr', null,
      el('td', { text: when(item.created_at) }),
      el('td', { text: item.actor || (item.ip_address === 'server-shell' ? 'сервер' : '—') }),
      el('td', null, ACTIONS[item.action] || item.action, item.success ? null : [' ', chip('отказ', 'off')]),
      el('td', { class: 'details', text: describe(item) }),
      el('td', { class: 'mono', text: item.ip_address || '' })));
  }

  async function loadAudit() {
    let answer;
    try {
      answer = await api('GET', '/api/admin/audit?limit=200');
    } catch (error) {
      report(error);
      return;
    }
    $('audit').tBodies[0].replaceChildren(...auditRows(answer.items || []));
    $('audit-empty').hidden = (answer.items || []).length > 0;
  }

  function showView(view) {
    state.view = view;
    for (const tab of document.querySelectorAll('.tab')) tab.classList.toggle('active', tab.dataset.view === view);
    for (const name of ['users', 'workspaces', 'audit']) $(`page-${name}`).hidden = name !== view;
    if (view === 'audit') loadAudit();
    if (view === 'workspaces') renderWorkspaces();
  }

  // ------------------------------------------------------------------ пароль

  function passwordRules() {
    const min = state.meta.password.min_length;
    return [
      [`не короче ${min} символов`, (text) => text.length >= min],
      ['заглавная буква', (text) => /\p{Lu}/u.test(text)],
      ['строчная буква', (text) => /\p{Ll}/u.test(text)],
      ['цифра', (text) => /\p{Nd}/u.test(text)],
      ['знак: не буква и не цифра', (text) => /[^\p{L}\p{N}]/u.test(text)],
    ];
  }

  // Случайное целое от 0 до limit - 1 без перекоса.
  function randomBelow(limit) {
    const ceiling = 256 - (256 % limit);
    const byte = new Uint8Array(1);
    do { crypto.getRandomValues(byte); } while (byte[0] >= ceiling);
    return byte[0] % limit;
  }

  function generatePassword() {
    const sets = ['ABCDEFGHJKLMNPQRSTUVWXYZ', 'abcdefghijkmnpqrstuvwxyz', '23456789', '!#$%*+-=?@'];
    const all = sets.join('');
    const pick = (alphabet) => alphabet[randomBelow(alphabet.length)];
    // По одному знаку каждого вида, остальное из всех; затем перемешать.
    const chars = sets.map(pick);
    while (chars.length < 16) chars.push(pick(all));
    for (let i = chars.length - 1; i > 0; i -= 1) {
      const j = randomBelow(i + 1);
      [chars[i], chars[j]] = [chars[j], chars[i]];
    }
    return chars.join('');
  }

  // Поля нового пароля с подсказкой требований. Пароль не покидает эту форму иначе как запросом к серверу.
  function passwordInputs() {
    const rules = passwordRules();
    const first = el('input', { type: 'password', autocomplete: 'new-password', maxLength: 200 });
    const second = el('input', { type: 'password', autocomplete: 'new-password', maxLength: 200 });
    const items = rules.map(([title]) => el('li', { text: title }));
    const match = el('li', { text: 'повтор совпадает' });
    const show = el('input', { type: 'checkbox' });
    const refresh = () => {
      rules.forEach(([, test], index) => items[index].classList.toggle('met', test(first.value)));
      match.classList.toggle('met', first.value !== '' && first.value === second.value);
    };
    first.addEventListener('input', refresh);
    second.addEventListener('input', refresh);
    show.addEventListener('change', () => {
      first.type = show.checked ? 'text' : 'password';
      second.type = first.type;
    });
    const generate = el('button', {
      class: 'btn small', type: 'button', text: 'Сгенерировать',
      on: {
        click: () => {
          first.value = generatePassword();
          second.value = first.value;
          show.checked = true;
          first.type = 'text';
          second.type = 'text';
          refresh();
        },
      },
    });
    const node = el('div', null,
      el('div', { class: 'two' }, field('Новый пароль', first), field('Повтор', second)),
      el('div', { class: 'row' }, el('label', { class: 'check' }, show, 'Показать'), generate),
      el('ul', { class: 'rules' }, items, match));
    return {
      node,
      // Пароль, если он годится, иначе null с объяснением.
      read() {
        if (first.value !== second.value) return { error: 'Пароль и повтор не совпадают.' };
        if (!rules.every(([, test]) => test(first.value))) return { error: TEXT.weak_password };
        return { value: first.value };
      },
      clear() {
        first.value = '';
        second.value = '';
        refresh();
      },
    };
  }

  // ------------------------------------------------------------------ доступ: черновик и редактор

  const sameList = (a, b) => a.length === b.length && a.every((value, index) => value === b[index]);
  const samePeriods = (a, b) => a.length === b.length && a.every((item, index) => item.from === b[index].from && item.to === b[index].to);

  function draftOf(user) {
    const roles = (user && user.workspace_roles) || {};
    const workspaces = {};
    for (const workspace of state.meta.workspaces) {
      // Роль в Чеках помнится и без доступа: при возврате доступа она та же.
      const kept = workspace.id === 'checks' && user ? user.role : workspace.default_role;
      workspaces[workspace.id] = {
        granted: user ? Boolean(roles[workspace.id]) : workspace.id === 'checks',
        role: roles[workspace.id] || kept || workspace.default_role,
      };
    }
    return {
      display_name: user ? user.display_name || '' : '',
      access_until: user ? dayValue(user.access_until) : '',
      access_note: user ? user.access_note || '' : '',
      session_ttl_days: user ? user.session_ttl_days : state.meta.session_ttl_days.default,
      global_admin: user ? Boolean(user.global_admin) : false,
      workspaces,
      tabs: user ? [...user.allowed_tabs].filter((tab) => tab !== 'admin') : ['dashboard'],
      folders: user ? [...user.allowed_folders] : [],
      sources: user ? user.allowed_sources.join(', ') : '',
      can_toggle_sources: user ? Boolean(user.can_toggle_sources) : false,
      periods: user ? user.forbidden_periods.map((item) => ({ from: item.from || '', to: item.to || '' })) : [],
    };
  }

  const grantOf = (entry) => (entry.granted ? entry.role : null);

  function parseSources(text) {
    const parts = text.split(/[\s,;]+/).filter(Boolean);
    if (!parts.every((part) => /^-?\d+$/.test(part))) return null;
    return [...new Set(parts.map(Number))];
  }

  // Ограничения оператора Чеков из черновика, как их принимает сервер; или текст ошибки.
  function checksRestrictions(draft) {
    const sources = parseSources(draft.sources);
    if (sources === null) return { error: 'Источники: только числовые ID чатов через запятую.' };
    const periods = draft.periods.filter((item) => item.from || item.to);
    if (periods.some((item) => !item.from || !item.to)) return { error: 'Закрытые периоды: укажите обе даты или удалите строку.' };
    const order = state.meta.checks.tabs.map((tab) => tab.id);
    return {
      value: {
        allowed_tabs: order.filter((id) => draft.tabs.includes(id)),
        allowed_folders: [...draft.folders].sort((a, b) => a - b),
        allowed_sources: sources,
        can_toggle_sources: draft.can_toggle_sources,
        forbidden_periods: periods.map((item) => ({ from: item.from, to: item.to })),
      },
    };
  }

  function checkGrid(items, selected, locked) {
    return el('div', { class: 'checks' }, items.map((item) => {
      const box = el('input', {
        type: 'checkbox',
        checked: selected.includes(item.id),
        disabled: locked === item.id,
        on: {
          change: () => {
            const at = selected.indexOf(item.id);
            if (box.checked && at < 0) selected.push(item.id);
            if (!box.checked && at >= 0) selected.splice(at, 1);
          },
        },
      });
      return el('label', { class: 'check' }, box, item.title);
    }));
  }

  function periodsEditor(periods) {
    const list = el('div');
    const draw = () => {
      list.replaceChildren(...periods.map((item, index) => el('div', { class: 'period' },
        el('input', { type: 'date', value: item.from, on: { change: (event) => { item.from = event.target.value; } }, attrs: { 'aria-label': 'С' } }),
        '—',
        el('input', { type: 'date', value: item.to, on: { change: (event) => { item.to = event.target.value; } }, attrs: { 'aria-label': 'По' } }),
        el('button', { class: 'btn small', type: 'button', text: 'Убрать', on: { click: () => { periods.splice(index, 1); draw(); } } }))));
    };
    draw();
    return el('div', null, list, el('div', { class: 'row' },
      el('button', { class: 'btn small', type: 'button', text: 'Добавить период', on: { click: () => { periods.push({ from: '', to: '' }); draw(); } } })));
  }

  // Данные Чеков, которыми ограничен оператор. У других рабочих пространств есть только роль.
  function checksEditor(draft) {
    if (!draft.tabs.includes('dashboard')) draft.tabs.unshift('dashboard');
    const folders = state.meta.checks.folders.map((folder) => ({
      id: folder.id,
      title: folder.is_active ? folder.name : `${folder.name} (отключена)`,
    }));
    const toggle = el('input', { type: 'checkbox', checked: draft.can_toggle_sources, on: { change: () => { draft.can_toggle_sources = toggle.checked; } } });
    const sources = el('input', { value: draft.sources, placeholder: 'например: -1001234567890, -1009876543210', on: { input: () => { draft.sources = sources.value; } } });
    return el('div', null,
      el('div', { class: 'sub' }, el('div', { class: 'label', text: 'Вкладки' }),
        checkGrid(state.meta.checks.tabs, draft.tabs, 'dashboard')),
      el('div', { class: 'sub' }, el('div', { class: 'label', text: 'Папки с данными' }),
        folders.length ? checkGrid(folders, draft.folders) : el('p', { class: 'hint', text: 'Папок на сервере нет.' }),
        el('p', { class: 'hint', text: 'Оператор видит данные только выбранных папок. Без папок он не увидит ничего.' })),
      el('div', { class: 'sub' }, el('div', { class: 'label', text: 'Источники: ID чатов Telegram' }), sources,
        el('p', { class: 'hint', text: 'Чеки из этих чатов доступны оператору. Пусто — источники Telegram недоступны.' }),
        el('label', { class: 'check' }, toggle, 'Может переключать источники')),
      el('div', { class: 'sub' }, el('div', { class: 'label', text: 'Закрытые периоды' }), periodsEditor(draft.periods),
        el('p', { class: 'hint', text: 'Данные за эти даты оператору не показываются.' })));
  }

  function accessEditor(draft, self) {
    const admin = el('input', { type: 'checkbox', checked: draft.global_admin, disabled: self && draft.global_admin, on: { change: () => { draft.global_admin = admin.checked; } } });
    const blocks = state.meta.workspaces.map((workspace) => {
      const entry = draft.workspaces[workspace.id];
      const hint = el('p', { class: 'hint' });
      const select = el('select', { attrs: { 'aria-label': `Роль: ${workspace.title}` } },
        workspace.roles.map((role) => el('option', { value: role.id, text: role.title })));
      select.value = entry.role;
      const granted = el('input', { type: 'checkbox', checked: entry.granted });
      const restrictions = workspace.id === 'checks' ? checksEditor(draft) : null;
      const unrestricted = restrictions
        ? el('p', { class: 'note', text: 'Администратор Чеков видит все данные Чеков: вкладки, папки, источники и периоды его не ограничивают.' })
        : null;
      const sync = () => {
        select.disabled = !entry.granted;
        const role = workspace.roles.find((item) => item.id === entry.role);
        hint.textContent = entry.granted ? (role ? role.hint : '') : 'Доступа нет.';
        if (restrictions) {
          restrictions.hidden = !(entry.granted && entry.role === 'operator');
          unrestricted.hidden = !(entry.granted && entry.role === 'admin');
        }
      };
      granted.addEventListener('change', () => { entry.granted = granted.checked; sync(); });
      select.addEventListener('change', () => { entry.role = select.value; sync(); });
      sync();
      return el('div', { class: 'ws' },
        el('div', { class: 'ws-head' }, el('label', { class: 'check' }, granted, workspace.title), select),
        el('div', { class: 'ws-body' }, hint,
          workspace.available ? null : el('p', { class: 'note', text: 'На сервере это рабочее пространство пока не подключено. Доступ можно выдать заранее; раздел откроется после подключения.' }),
          unrestricted, restrictions));
    });
    return [
      section('Администрирование CRM',
        el('label', { class: 'check' }, admin, 'Администратор CRM'),
        el('p', { class: 'hint', text: 'Создаёт учётные записи, выдаёт и отзывает доступ, завершает сессии, меняет настройки безопасности системы. Сам по себе не открывает ни одного рабочего пространства.' }),
        self && draft.global_admin ? el('p', { class: 'note', text: 'Это ваша учётная запись: снять права администратора с себя нельзя.' }) : null),
      section('Рабочие пространства', el('p', { class: 'hint', text: 'Отметьте, что человек может открывать, и выберите роль. Роль в одном пространстве ничего не даёт в другом.' }), blocks),
    ];
  }

  // ------------------------------------------------------------------ карточка

  function openSheet(title, chips, body, dirty) {
    $('sheet-title').textContent = title;
    $('sheet-access').replaceChildren(...chips);
    $('sheet-body').replaceChildren(...body);
    $('sheet-body').scrollTop = 0;
    $('sheet').hidden = false;
    $('backdrop').hidden = false;
    state.sheet = { dirty };
  }

  async function closeSheet(force) {
    if ($('sheet').hidden) return;
    if (force !== true && state.sheet && state.sheet.dirty && state.sheet.dirty()) {
      const sure = await confirmAction('Закрыть без сохранения?', 'Изменения в карточке не сохранены.', 'Закрыть');
      if (!sure) return;
    }
    $('sheet').hidden = true;
    $('backdrop').hidden = true;
    $('sheet-body').replaceChildren();
    state.sheet = null;
  }

  function effectiveLines(user) {
    const effective = user.effective_access || { summary: [], workspaces: [] };
    const lines = effective.summary.map((text) => el('li', { text }));
    for (const item of effective.workspaces || []) {
      const workspace = workspaceOf(item.id);
      if (item.effective && !item.in_hub) lines.push(el('li', { class: 'muted', text: `${item.title}: раздела в Hub для этой роли нет.` }));
      if (item.effective && workspace && !workspace.available) lines.push(el('li', { class: 'muted', text: `${item.title}: на сервере не подключено, раздел пока не откроется.` }));
    }
    return el('ul', { class: 'lines' }, lines);
  }

  function changesOf(user, draft, original) {
    const changes = {};
    for (const key of ['display_name', 'access_until', 'access_note', 'session_ttl_days', 'global_admin']) {
      if (draft[key] !== original[key]) changes[key] = draft[key];
    }
    const grants = {};
    for (const workspace of state.meta.workspaces) {
      const now = grantOf(draft.workspaces[workspace.id]);
      if (now !== grantOf(original.workspaces[workspace.id])) grants[workspace.id] = now;
    }
    if (Object.keys(grants).length) changes.workspaces = grants;

    if (grantOf(draft.workspaces.checks || {}) === 'operator') {
      const restrictions = checksRestrictions(draft);
      if (restrictions.error) return { error: restrictions.error };
      const before = checksRestrictions(original).value;
      const now = restrictions.value;
      // Ограничения администратора Чеков сервер не показывает. Когда он становится
      // оператором, сохраняется ровно то, что видно в карточке.
      const all = user.role === 'admin';
      if (all || !sameList(now.allowed_tabs, before.allowed_tabs)) changes.allowed_tabs = now.allowed_tabs;
      if (all || !sameList(now.allowed_folders, before.allowed_folders)) changes.allowed_folders = now.allowed_folders;
      if (all || !sameList(now.allowed_sources, before.allowed_sources)) changes.allowed_sources = now.allowed_sources;
      if (all || now.can_toggle_sources !== before.can_toggle_sources) changes.can_toggle_sources = now.can_toggle_sources;
      if (all || !samePeriods(now.forbidden_periods, before.forbidden_periods)) changes.forbidden_periods = now.forbidden_periods;
    }
    return { value: changes };
  }

  function sessionsBlock(user) {
    const list = el('ul', { class: 'sessions' }, el('li', { class: 'muted', text: 'Загрузка…' }));
    const load = async () => {
      let answer;
      try {
        answer = await api('GET', `/api/admin/users/${user.id}/sessions`);
      } catch (error) {
        if (!error.handled) list.replaceChildren(el('li', { class: 'muted', text: messageOf(error) }));
        return;
      }
      const sessions = answer.sessions || [];
      list.replaceChildren(...(sessions.length ? sessions.map((item) => el('li', null,
        el('span', { class: 'mono', text: item.ip_address || 'адрес неизвестен' }),
        ` · вход ${when(item.created_at)} · активность ${when(item.last_activity_at)} · до ${when(item.expires_at)}`,
        item.current ? [' ', chip('эта сессия', 'ok')] : null))
        : [el('li', { class: 'muted', text: 'Открытых сессий нет.' })]));
    };
    load();
    const end = el('button', {
      class: 'btn quiet-danger', type: 'button', text: 'Завершить все сессии',
      on: {
        click: async () => {
          const sure = await confirmAction('Завершить сессии?', `${user.username} будет выведен из всех клиентов и войдёт заново со своим паролем. Права и доступ не меняются.`, 'Завершить');
          if (!sure) return;
          try {
            const answer = await api('POST', `/api/admin/users/${user.id}/revoke-sessions`, {});
            toast(answer.kept_current ? `Завершено сессий: ${answer.revoked}. Текущая оставлена.` : `Завершено сессий: ${answer.revoked}.`);
            load();
          } catch (error) {
            report(error);
          }
        },
      },
    });
    return section('Сессии', list, el('div', { class: 'row' }, end));
  }

  function journalBlock(user) {
    const body = el('tbody');
    const table = el('table', { class: 'grid', hidden: true }, body);
    const button = el('button', {
      class: 'btn', type: 'button', text: 'Показать действия по этой записи',
      on: {
        click: async () => {
          try {
            const answer = await api('GET', `/api/admin/audit?limit=50&user_id=${user.id}`);
            const rows = auditRows(answer.items || []);
            body.replaceChildren(...(rows.length ? rows : [el('tr', null, el('td', { class: 'muted', text: 'Записей нет.' }))]));
            table.hidden = false;
          } catch (error) {
            report(error);
          }
        },
      },
    });
    return section('Журнал', el('div', { class: 'row' }, button), table);
  }

  // Действие над записью: запрос, свежая карточка, сообщение.
  async function act(user, method, path, body, done) {
    try {
      const updated = await api(method, `/api/admin/users/${user.id}${path}`, body);
      applyUser(updated);
      toast(done);
      openUser(updated.id);
    } catch (error) {
      report(error);
    }
  }

  function openUser(id) {
    const user = state.users.find((item) => item.id === id);
    if (!user) return;
    const self = user.id === state.meta.me.id;
    const draft = draftOf(user);
    const original = draftOf(user);
    const ttl = state.meta.session_ttl_days;

    const name = el('input', { value: draft.display_name, maxLength: 100, on: { input: () => { draft.display_name = name.value.trim(); } } });
    const until = el('input', { type: 'date', value: draft.access_until, on: { change: () => { draft.access_until = until.value; } } });
    const note = el('input', { value: draft.access_note, maxLength: 200, on: { input: () => { draft.access_note = note.value.trim(); } } });
    const days = el('input', { type: 'number', min: ttl.min, max: ttl.max, value: draft.session_ttl_days, on: { input: () => { draft.session_ttl_days = Number(days.value); } } });

    const save = el('button', { class: 'btn primary', type: 'button', text: 'Сохранить' });
    save.addEventListener('click', async () => {
      const changes = changesOf(user, draft, original);
      if (changes.error) {
        toast(changes.error, true);
        return;
      }
      if (!Object.keys(changes.value).length) {
        toast('Изменений нет.');
        return;
      }
      if (!Number.isInteger(draft.session_ttl_days) || draft.session_ttl_days < ttl.min || draft.session_ttl_days > ttl.max) {
        toast(`Срок сессии: от ${ttl.min} до ${ttl.max} дней.`, true);
        return;
      }
      save.disabled = true;
      await act(user, 'PATCH', '', changes.value, 'Сохранено. Изменения действуют со следующего запроса пользователя.');
      save.disabled = false;
    });

    const password = passwordInputs();
    const setPassword = el('button', {
      class: 'btn', type: 'button', text: 'Задать пароль',
      on: {
        click: async () => {
          const entered = password.read();
          if (entered.error) {
            toast(entered.error, true);
            return;
          }
          const sure = await confirmAction('Задать новый пароль?', `Старый пароль ${user.username} перестанет действовать, все его сессии завершатся.`, 'Задать пароль');
          if (!sure) return;
          await act(user, 'POST', '/reset-password', { password: entered.value }, 'Пароль изменён. Сессии пользователя завершены.');
        },
      },
    });

    const force = el('input', {
      type: 'checkbox', checked: user.force_2fa,
      on: { change: () => act(user, 'POST', '/force-2fa', { force: force.checked }, force.checked ? '2FA теперь обязательна при входе.' : 'Требование 2FA снято.') },
    });
    const reset2fa = user.totp_enabled ? el('button', {
      class: 'btn quiet-danger', type: 'button', text: 'Сбросить 2FA',
      on: {
        click: async () => {
          const sure = await confirmAction('Сбросить 2FA?', `Приложение-аутентификатор ${user.username} перестанет подходить. Если 2FA обязательна, при следующем входе её нужно настроить заново.`, 'Сбросить');
          if (sure) await act(user, 'POST', '/reset-2fa', {}, '2FA сброшена.');
        },
      },
    }) : null;

    const tgId = el('input', { inputMode: 'numeric', value: user.telegram_id || '', placeholder: 'например: 123456789' });
    const tgName = el('input', { value: user.telegram_username || '', placeholder: 'необязательно', maxLength: 255 });
    const link = el('button', {
      class: 'btn', type: 'button', text: user.telegram_id ? 'Изменить' : 'Привязать',
      on: {
        click: () => {
          const text = tgId.value.trim();
          if (!/^\d{1,19}$/.test(text) || Number(text) <= 0) {
            toast('Telegram ID — положительное число. ID группы или канала (с минусом) не подходит.', true);
            return;
          }
          act(user, 'POST', '/link-telegram', { telegram_id: Number(text), telegram_username: tgName.value.trim().replace(/^@/, '') || null }, 'Telegram привязан.');
        },
      },
    });
    const unlink = user.telegram_id ? el('button', {
      class: 'btn quiet-danger', type: 'button', text: 'Отвязать',
      on: { click: () => act(user, 'POST', '/unlink-telegram', {}, 'Telegram отвязан.') },
    }) : null;

    const lockedNow = Boolean((user.effective_access || {}).locked);
    const unlock = lockedNow || user.failed_attempts > 0 ? el('button', {
      class: 'btn', type: 'button', text: 'Снять блокировку входа',
      on: { click: () => act(user, 'POST', '/unlock', {}, 'Блокировка входа снята.') },
    }) : null;

    const toggleActive = user.is_active
      ? el('button', {
        class: 'btn danger', type: 'button', text: 'Отключить учётную запись', disabled: self,
        on: {
          click: async () => {
            const sure = await confirmAction('Отключить учётную запись?', `${user.username} не сможет войти ни в одно рабочее пространство, его сессии завершатся. Выданный доступ сохранится и вернётся при включении.`, 'Отключить');
            if (sure) await act(user, 'DELETE', '', undefined, 'Учётная запись отключена.');
          },
        },
      })
      : el('button', {
        class: 'btn primary', type: 'button', text: 'Включить учётную запись',
        on: { click: () => act(user, 'PATCH', '', { is_active: true }, 'Учётная запись включена.') },
      });

    openSheet(
      `${user.display_name || user.username} · ${user.username}`,
      [stateChip(user), ...accessChips(user)],
      [
        section('Действующий доступ', effectiveLines(user)),
        section('Учётная запись',
          el('div', { class: 'two' }, field('Имя', name), field('Срок сессии, дней', days)),
          el('div', { class: 'two' }, field('Доступ действует до', until), field('Примечание к сроку', note)),
          el('p', { class: 'hint', text: 'Пустая дата — без срока. На администраторов срок не действует.' })),
        ...accessEditor(draft, self),
        el('div', { class: 'savebar' }, save, el('span', { class: 'muted', text: 'Имя, срок, администрирование и рабочие пространства' })),
        section('Пароль', el('p', { class: 'hint', text: 'Действующий пароль нигде не показывается и не хранится в открытом виде. Можно только задать новый.' }),
          password.node, el('div', { class: 'row' }, setPassword)),
        section('Двухфакторная аутентификация',
          el('p', { text: user.totp_enabled ? `Настроена ${day(user.totp_confirmed_at)}.` : 'Не настроена.' }),
          el('label', { class: 'check' }, force, 'Требовать 2FA при входе'),
          reset2fa ? el('div', { class: 'row' }, reset2fa) : null),
        sessionsBlock(user),
        section('Telegram',
          el('div', { class: 'inline' }, field('Telegram ID', tgId), field('Имя в Telegram', tgName), link, unlink),
          el('p', { class: 'hint', text: 'ID вводит администратор: человек узнаёт его командой /id у бота. По этому ID бот учёта времени принимает отметки, имя для этого не используется.' })),
        section('Вход',
          el('p', { text: `Последний вход: ${when(user.last_login_at)}. Неудачных попыток подряд: ${user.failed_attempts}.${lockedNow ? ` Заблокирован до ${when(user.locked_until)}.` : ''}` }),
          unlock ? el('div', { class: 'row' }, unlock) : null),
        section('Состояние', el('div', { class: 'row' }, toggleActive),
          self ? el('p', { class: 'hint', text: 'Это ваша учётная запись: отключить её может другой администратор.' }) : null),
        journalBlock(user),
      ],
      () => {
        const changes = changesOf(user, draft, original);
        return Boolean(changes.error) || Object.keys(changes.value).length > 0;
      },
    );
  }

  function openCreate() {
    const draft = draftOf(null);
    const username = el('input', { maxLength: 50, autocomplete: 'off' });
    const name = el('input', { maxLength: 100, autocomplete: 'off' });
    const force = el('input', { type: 'checkbox' });
    const password = passwordInputs();
    const create = el('button', { class: 'btn primary', type: 'button', text: 'Создать' });
    create.addEventListener('click', async () => {
      const loginName = username.value.trim();
      if (!loginName) {
        toast('Укажите логин.', true);
        return;
      }
      const entered = password.read();
      if (entered.error) {
        toast(entered.error, true);
        return;
      }
      const grants = {};
      for (const workspace of state.meta.workspaces) {
        const role = grantOf(draft.workspaces[workspace.id]);
        if (role) grants[workspace.id] = role;
      }
      const body = {
        username: loginName,
        password: entered.value,
        display_name: name.value.trim() || null,
        force_2fa: force.checked,
        global_admin: draft.global_admin,
        workspaces: grants,
      };
      if (grants.checks === 'operator') {
        const restrictions = checksRestrictions(draft);
        if (restrictions.error) {
          toast(restrictions.error, true);
          return;
        }
        Object.assign(body, restrictions.value);
      }
      create.disabled = true;
      try {
        const user = await api('POST', '/api/admin/users', body);
        applyUser(user);
        toast(`Учётная запись ${user.username} создана.`);
        closeSheet(true);
        openUser(user.id);
      } catch (error) {
        report(error);
      } finally {
        create.disabled = false;
      }
    });

    openSheet('Новая учётная запись', [], [
      section('Учётная запись',
        el('div', { class: 'two' }, field('Логин', username), field('Имя', name)),
        el('p', { class: 'hint', text: 'Одна учётная запись на человека для всей CRM: вход один, рабочие пространства выдаются ниже.' })),
      section('Пароль', password.node,
        el('label', { class: 'check' }, force, 'Требовать 2FA при входе'),
        el('p', { class: 'hint', text: 'Пароль передайте человеку лично. Здесь он больше не будет показан.' })),
      ...accessEditor(draft, false),
      el('div', { class: 'savebar' }, create, el('button', { class: 'btn ghost', type: 'button', text: 'Отмена', on: { click: () => closeSheet(true) } })),
    ], () => Boolean(username.value.trim()));
    username.focus();
  }

  // ------------------------------------------------------------------ запуск

  function wire() {
    $('login-form').addEventListener('submit', submitLogin);
    $('login-back').addEventListener('click', () => showLogin(''));
    $('logout').addEventListener('click', async () => {
      await dropSession();
      leave('');
    });
    for (const tab of document.querySelectorAll('.tab')) tab.addEventListener('click', () => showView(tab.dataset.view));
    for (const id of ['search', 'filter-state', 'filter-access']) $(id).addEventListener('input', renderUsers);
    $('create').addEventListener('click', openCreate);
    $('audit-refresh').addEventListener('click', loadAudit);
    $('sheet-close').addEventListener('click', () => closeSheet());
    $('backdrop').addEventListener('click', () => closeSheet());
    document.addEventListener('keydown', (event) => {
      if (event.key !== 'Escape') return;
      if (!$('confirm').hidden) $('confirm-no').click();
      else closeSheet();
    });
  }

  wire();
  resume();
})();
