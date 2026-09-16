/* Box + L자 뚜껑 (위에서 나사 체결) */

// ===== 기본 박스 치수 =====
// L = 125;  // X
// L = 80;  // X - left
L = 170;  // X - right
W = 50;   // Y
H = 30;   // Z
t = 1.5;  // 두께

// ===== 뚜껑 파라미터 =====
plate_t  = 2;   // 상판 두께
flap_t   = 1.5;   // -Y 측판 두께
flap_bot = 3;   // 측판 아래 끝 Z
screw_d  = 3.2; // 나사 구멍 지름
SHOW_BOX = false; // STL 뽑을 때는 false

screw_xys = [
  [-L/2+4, -W/2+4], [ L/2-4, -W/2+4],
  [-L/2+4,  W/2-4], [ L/2-4,  W/2-4]
];

// ===== 박스 =====
module wire_box() {
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
        cylinder(H+2, 2, 2, center=true, $fn=48);
    }
}

// ===== 뚜껑 =====
module box_lid() {
    difference() {
        union() {
            // 상판: 윗면 전체 커버 (-Y 쪽은 측판과 일체)
            translate([-L/2, -W/2 - flap_t, H])
                cube([L, W + flap_t, plate_t]);
            // -Y 측판: 열린 면을 바깥에서 덮음
            translate([-L/2, -W/2 - flap_t, flap_bot+10])
                cube([L, flap_t, H + 1 - flap_bot-9]);
        }
        // 위에서 체결하는 나사 구멍 φ3.2 (코너 필라 중심)
        for (xy = screw_xys)
            translate([xy[0], xy[1], H - 1])
                cylinder(plate_t + 2, d=screw_d, $fn=48);
    }
}

if (SHOW_BOX) wire_box();
box_lid();
