"""
Import operators/sellers from text file to database
"""
from database.connection import get_db
from database.models import OperatorReference
import sys
import os

def import_operators_from_file(file_path: str):
    """Import operators from text file"""
    if not os.path.exists(file_path):
        print(f"❌ File not found: {file_path}")
        return False

    with get_db() as db:
        # Clear existing data
        print("🗑️  Clearing existing data...")
        db.query(OperatorReference).delete()
        db.commit()

        # Read file
        print(f"📖 Reading file: {file_path}")
        with open(file_path, 'r', encoding='utf-8') as f:
            lines = f.readlines()

        imported = 0
        skipped = 0

        for line_num, line in enumerate(lines, 1):
            line = line.strip()

            # Skip empty lines and headers
            if not line or 'оператор/продавец' in line.lower():
                continue

            # Parse line: "Operator — Application"
            if ' — ' in line:
                parts = line.split(' — ')
                if len(parts) == 2:
                    operator_name = parts[0].strip()
                    application_name = parts[1].strip()

                    # Create record
                    try:
                        operator_ref = OperatorReference(
                            operator_name=operator_name,
                            application_name=application_name,
                            is_p2p=True,  # All records in file are P2P
                            is_active=True
                        )
                        db.add(operator_ref)
                        imported += 1

                        if imported % 10 == 0:
                            print(f"✓ Imported {imported} records...")

                    except Exception as e:
                        print(f"⚠️  Line {line_num}: Error - {e}")
                        skipped += 1
                else:
                    print(f"⚠️  Line {line_num}: Invalid format - {line}")
                    skipped += 1
            else:
                skipped += 1

        # Commit all changes
        db.commit()

        print(f"\n✅ Import completed!")
        print(f"   Imported: {imported}")
        print(f"   Skipped: {skipped}")

        # Verify
        total = db.query(OperatorReference).count()
        print(f"   Total in DB: {total}")

        return True


if __name__ == "__main__":
    file_path = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "ops.txt")
    if len(sys.argv) > 1:
        file_path = sys.argv[1]

    import_operators_from_file(file_path)
