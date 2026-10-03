import SwiftUI

struct NotchStatusView: View {
    let status: NotchStatus
    let notch: CGSize
    private var color: Color { status.kind == .focus ? .buddyIndigo : status.active == false ? .secondary : .white }

    var body: some View {
        HStack(spacing: 0) {
            HStack(spacing: 6) {
                Image(systemName: status.symbol).font(.system(size: 14, weight: .medium)).foregroundStyle(color)
                Text(status.title).font(.system(size: 10, weight: .medium)).lineLimit(1).truncationMode(.tail)
            }.frame(width: NotchLayout.statusWing - NotchLayout.compactInset, alignment: .trailing)
            Color.clear.frame(width: notch.width)
            Group {
                if let level = status.level {
                    GeometryReader { geometry in
                        Capsule().fill(.white.opacity(0.18))
                            .overlay(alignment: .leading) { Capsule().fill(.white).frame(width: geometry.size.width * min(max(level, 0), 1)) }
                    }.frame(width: 72, height: 5)
                } else if let battery = status.battery, status.active != false {
                    ZStack {
                        Circle().stroke(.white.opacity(0.2), lineWidth: 2)
                        Circle().trim(from: 0, to: min(max(battery, 0), 1)).stroke(Color.accentColor, style: StrokeStyle(lineWidth: 2, lineCap: .round)).rotationEffect(.degrees(-90))
                    }.frame(width: 16, height: 16)
                } else if status.kind == .connection {
                    Image(systemName: status.active == false ? "xmark.circle" : "checkmark.circle")
                        .font(.system(size: 16, weight: .medium)).foregroundStyle(status.active == false ? Color.secondary : Color.accentColor)
                } else {
                    Text(status.text).font(.system(size: 10, weight: .semibold)).foregroundStyle(color).lineLimit(1)
                }
            }.frame(width: NotchLayout.statusWing - NotchLayout.compactInset, alignment: .leading)
        }.padding(.horizontal, NotchLayout.compactInset).contentShape(Rectangle()).tip(status.help).accessibilityElement(children: .ignore).accessibilityLabel(status.help)
    }
}
