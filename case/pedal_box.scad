difference() {
    pedal_box();
    rotate([15.95,0,0]) translate([0,65,0]) #oled();
    #bottom_holes();
}


module pedal_box() {
    // Shell: outer profile minus inner cavity (1.5mm walls), open at x=100 face
    H = 65;
    T = 1.5;

    translate([-H/2,0,0]) rotate([90,0,90])
    difference() {
        linear_extrude(height = H)
            polygon(points = [[0, 0], [0, -10], [100, -10], [100, 20], [70, 20]]);

        // cavity = outer profile offset inward by 1.5, extended past x=100 to open that face
        // hypotenuse (0,0)-(70,20) offset: 2x - 7y = 1.5*sqrt(53) ≈ 10.920
        translate([0,0,T]) linear_extrude(height = H-T*2)
            polygon(points = [
                [1.5, -1.1315],   // offset hypotenuse x offset x=1.5
                [1.5, -8.5],      // offset hypotenuse-> no: corner x=1.5 & y=-8.5
                [101.5, -8.5],    // past x=100 -> opens the face
                [101.5, 18.5],
                [70.2101, 18.5]   // offset hypotenuse x offset y=18.5
            ]);
    }
}


module oled() {
    hw=33.8-3;
    hh=32-3;
    
    rotate([0,0,180]) {
        translate([-hw/2,0,0]) union() {
            cylinder(h=20, r=3.2/2, center=true, $fn=20);
            translate([hw, hh, 0]) cylinder(h=20, r=3.2/2, center=true, $fn=20);
            translate([hw, 0, 0]) cylinder(h=20, r=3.2/2, center=true, $fn=20);
            translate([0, hh, 0]) cylinder(h=20, r=3.2/2, center=true, $fn=20);
        }
        translate([-32/2,4,-10]) cube([32,17,20]);
    }
}

module bottom_holes() {
    // w=50, h=80
    translate([25,10,-15]) cylinder(10, 3, 3);
    translate([-25,10,-15]) cylinder(10, 3, 3);
    translate([25,90,-15]) cylinder(10, 3, 3);
    translate([-25,90,-15]) cylinder(10, 3, 3);
}