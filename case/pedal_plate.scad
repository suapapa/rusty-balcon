
translate([-270/2,0,0]) cube([270, 110, 1]);

translate([-70*2+70/2,0,0]) union(){
    pedal();
    translate([70,0,0]) control_box();
    translate([70*2,0,0]) pedal();
    translate([70*3,0,0]) pedal();
}

difference() {
    union() {
        translate([-65,100+25+5,0]) wire_box();
        translate([65,100+25+5,0]) wire_box();
    }
    translate([0,125,15])rotate([0,90,0]) cylinder(h=30,r=5,center=true);
    translate([125,125+15,15]) usb_c();
    translate([125,125-3,15]) pwr_sw();
}

module pedal() {
    w = 46.4;   // 사각형 가로 (X)
    h = 80.8;   // 사각형 세로 (Y)
    r = 3;      // 실린더 반지름
    hgt = 7;    // 실린더 높이
    $fn = 12;
    translate([0,h/2+8,0]) {
        for (x = [-w/2, w/2], y = [-h/2, h/2]) {
            translate([x, y, 0])
                cylinder(r = r, h = hgt);
        }
        
        hull() {
            translate([-w/2, -h/2, 0]) cylinder(r=r+2, h=2);
            translate([w/2, h/2, 0]) cylinder(r=r+2, h=2);
        }
        hull() {
            translate([-w/2, h/2, 0]) cylinder(r=r+2, h=2);
            translate([w/2, -h/2, 0]) cylinder(r=r+2, h=2);
        }
        
        translate([0,0,1]) cube([26,43,2], center=true);
    }
}

module control_box() {
    r=3-0.1;
    translate([25,10,0]) cylinder(5, r, r);
    translate([-25,10,0]) cylinder(5, r, r);
    translate([25,90,0]) cylinder(5, r, r);
    translate([-25,90,0]) cylinder(5, r, r);
}

module wire_box() {
    L = 125;  // X
    W = 50;   // Y
    H = 30;   // Z
    t = 1.5;  // 두께

    difference() {
        translate([-L/2, -W/2, 0])
            cube([L, W, H]);
        translate([-L/2 + t, -W/2 + t, t])
            cube([L - 2*t, W - 2*t, H - t + 1]);
        translate([-L/2 + t, -W/2 + t-5, t+5])
            cube([L - 2*t, W - 2*t, H - t + 1]);
    }
    
    for (x = [-L/2+4, L/2-4], y = [-W/2+4, W/2-4]) {
        translate([x,y,0]) box_pilla(H);
    }

}

module box_pilla(H) {
    translate([0,0,H/2]) difference() {
        cube([8,8,H], center=true);
        cylinder(H, 2, 2, $fn=10);
    }
}

module usb_c(){
    rotate([0,90,0]) union() {
        cube([13.5,5.5,20], center=true);
        cube([14,3.5,20], center=true);        
    }
}

module pwr_sw() {
    rotate([90,0,0]) rotate([0,90,0]) union() {
        cylinder(h=20,r=15.2/2,center=true,$fn=20);
        cube([4,15.5,30],center=true);
    }
}