<?php
echo number_format(1.005, 2), "\n";
echo number_format(-1.005, 2), "\n";
echo number_format(1234567.891, 2), "\n";
echo number_format(1234567.891, 2, ",", "."), "\n";
echo number_format(1.2345678901234567, 30), "\n";
echo number_format(0.1, 30), "\n";
echo number_format(1.5, 1000), "\n";
echo sprintf("%.1000f", 1.2345678901234567), "\n";
echo sprintf("%.1000e", 1.2345678901234567), "\n";
echo sprintf("%.1000g", 1.2345678901234567), "\n";
