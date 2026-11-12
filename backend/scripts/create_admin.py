"""Bootstrap a CRM administrator from the server's own shell.

The administration panel is opened by a CRM administrator, so the first one
cannot come from the panel. This script is that first step, and the way back
in if the last one is ever lost. It needs what only the server's owner has:
a shell on the machine and the database credentials.

Usage:
    python scripts/create_admin.py <username> [--name "Display name"]
        Creates the account, asking for the password without echo. In a
        non-interactive run the password is read from the CRM_ADMIN_PASSWORD
        environment variable.
    python scripts/create_admin.py --promote <username>
        Makes an existing account a CRM administrator. Its password, its
        workspaces and everything else about it stay as they are.

    python scripts/create_admin.py <username> <password> [display_name]
        The older form still works. It leaves the password in the shell
        history and the process list, so prefer the prompt.

The account created here is a CRM administrator and, as it always was, a
Checks administrator. Other workspaces are granted in the panel.
"""
from __future__ import annotations

import argparse
import getpass
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from database.connection import SessionLocal
from services.access_control_service import write_audit_log
from services.user_service import (
    ALL_TABS,
    UserServiceError,
    create_user,
    get_user_by_username,
    update_user,
)

PASSWORD_ENV = "CRM_ADMIN_PASSWORD"


def _audit(db, action: str, user) -> None:
    # In the same journal the panel shows, so an administrator who appeared
    # from the shell is not a mystery later. No password is written.
    try:
        write_audit_log(
            db,
            action=action,
            success=True,
            ip_address="server-shell",
            details={"user_id": int(user.id), "username": user.username},
        )
    except Exception:
        db.rollback()


def create_admin(username: str, password: str, display_name: str = "Админ") -> int:
    db = SessionLocal()
    try:
        existing = get_user_by_username(db, username)
        if existing:
            print(f"User '{existing.username}' already exists (id={existing.id}); nothing was changed.")
            print("To make it a CRM administrator: python scripts/create_admin.py --promote " + existing.username)
            return int(existing.id)

        user = create_user(
            db,
            username=username,
            password=password,
            role="admin",
            display_name=display_name,
            allowed_tabs=list(ALL_TABS),
            can_toggle_sources=True,
            global_admin=True,
        )
        _audit(db, "admin_bootstrap_create", user)
        print(f"CRM administrator '{user.username}' created (id={user.id})")
        return int(user.id)
    finally:
        db.close()


def promote_admin(username: str) -> int:
    db = SessionLocal()
    try:
        user = get_user_by_username(db, username)
        if not user:
            print(f"User '{username}' not found")
            raise SystemExit(1)
        if not user.is_active:
            print(f"User '{user.username}' is disabled; enable it first or choose another account")
            raise SystemExit(1)
        update_user(db, user, global_admin=True)
        _audit(db, "admin_bootstrap_promote", user)
        print(f"'{user.username}' (id={user.id}) is now a CRM administrator")
        return int(user.id)
    finally:
        db.close()


def _read_password() -> str:
    from_env = os.environ.get(PASSWORD_ENV)
    if from_env:
        return from_env
    if not sys.stdin.isatty():
        print(f"No terminal to ask for the password: set {PASSWORD_ENV} for this run")
        raise SystemExit(1)
    first = getpass.getpass("Password: ")
    if first != getpass.getpass("Repeat password: "):
        print("The passwords differ")
        raise SystemExit(1)
    return first


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Create the first CRM administrator, or make an existing account one.",
    )
    parser.add_argument("username")
    parser.add_argument("password", nargs="?", help="older form; prefer the prompt")
    parser.add_argument("display_name", nargs="?")
    parser.add_argument("--name", help="display name when the password is asked for")
    parser.add_argument(
        "--promote",
        action="store_true",
        help="make the existing account a CRM administrator; nothing else about it changes",
    )
    args = parser.parse_args()

    username = args.username.strip()
    if not username:
        parser.error("username is empty")

    if args.promote:
        if args.password or args.display_name or args.name:
            parser.error("--promote takes the username and nothing else")
        promote_admin(username)
        return

    password = args.password if args.password is not None else _read_password()
    if not password:
        parser.error("password is empty")
    display_name = (args.display_name or args.name or "Админ").strip() or "Админ"

    try:
        create_admin(username=username, password=password, display_name=display_name)
    except UserServiceError as exc:
        print(f"Refused: {exc}")
        raise SystemExit(1)


if __name__ == "__main__":
    main()
