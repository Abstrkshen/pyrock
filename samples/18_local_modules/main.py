# Importing your own .py files: they are compiled natively into the same binary.
import geometry
from stats import mean, variance

print("geometry loaded:", geometry.name())
print("circle area:", geometry.circle_area(1.5))
print("rectangle:", geometry.rect_area(3, 4), geometry.rect_area(2.5, 2))
print("mean:", mean(2.0, 4.0, 9.0), "variance:", variance(2.0, 4.0, 9.0))
