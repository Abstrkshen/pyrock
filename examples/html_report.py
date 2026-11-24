# mode: native-only
# Print HTML and the playground renders it.
from dataclasses import dataclass

@dataclass
class Order:
    customer: str
    items: int
    price: float

orders = [Order("Ada", 3, 12.5), Order("Grace", 1, 99.0), Order("Linus", 7, 4.25), Order("Guido", 2, 30.0)]
total = sum(o.items * o.price for o in orders)

rows = []
for o in sorted(orders, key=lambda o: -o.items * o.price):
    amount = o.items * o.price
    width = round(100 * amount / total)
    rows.append(f"<tr><td>{o.customer}</td><td>{o.items}</td><td>{amount:.2f}</td>"
                f'<td><div style="background:#0f6e57;height:10px;width:{width}%"></div></td></tr>')

print(f"""<!doctype html>
<style>
  body {{ font-family: system-ui, sans-serif; margin: 24px; color: #1c2326; }}
  table {{ border-collapse: collapse; width: 100%; }}
  th, td {{ text-align: left; padding: 8px 10px; border-bottom: 1px solid #ccd; }}
  td:last-child {{ width: 40%; }}
</style>
<h2>Orders</h2>
<table>
<tr><th>Customer</th><th>Items</th><th>Amount</th><th>Share</th></tr>
{"".join(rows)}
</table>
<p>Total: <strong>{total:.2f}</strong> from {len(orders)} orders.</p>""")
