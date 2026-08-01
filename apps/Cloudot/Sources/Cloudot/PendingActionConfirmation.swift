import SwiftUI

struct PendingActionConfirmation: ViewModifier {
    @Bindable var model: AppModel

    func body(content: Content) -> some View {
        content.alert(
            model.pending?.title ?? "",
            isPresented: $model.isPresentingPendingAction,
            presenting: model.pending
        ) { action in
            Button(
                action.confirmLabel,
                role: action.isDestructive ? .destructive : nil
            ) {
                Task {
                    await model.confirm(action)
                }
            }
            Button("取消", role: .cancel) {
                model.pending = nil
            }
        } message: { action in
            Text(action.explanation)
        }
    }
}

extension View {
    /// GUI 里所有会改文件的操作都经过确认。
    func confirm(_ model: AppModel) -> some View {
        modifier(PendingActionConfirmation(model: model))
    }
}
