# pyrock run main.py
import shapes
import units

print(shapes.name())
print("ring:", shapes.ring_area(3.0, 1.0))
print("scaled radius mm:", units.millimeters(shapes.diameter(1.5)))
print("double inches:", units.millimeters(units.double(1.0)))
