// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI
import UIKit

/// SwiftUI's automatic drawer can start expanded with an inline title/short
/// list. Collapse it once after the initial data and native layout are ready.
/// UIKit still owns all subsequent pull-down, keyboard, and cancel behavior.
struct InitiallyCollapsedSearch: UIViewControllerRepresentable {
    let ready: Bool
    let queryIsEmpty: Bool

    func makeUIViewController(context: Context) -> Controller { Controller() }

    func updateUIViewController(_ controller: Controller, context: Context) {
        controller.ready = ready
        controller.queryIsEmpty = queryIsEmpty
        controller.scheduleCollapse()
    }

    final class Controller: UIViewController {
        var ready = false
        var queryIsEmpty = true
        private var collapsed = false
        private var scheduled = false

        override func loadView() {
            view = UIView()
            view.isUserInteractionEnabled = false
        }

        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            scheduleCollapse()
        }

        override func viewDidLayoutSubviews() {
            super.viewDidLayoutSubviews()
            scheduleCollapse()
        }

        func scheduleCollapse() {
            guard ready, !collapsed, !scheduled else { return }
            scheduled = true
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.scheduled = false
                self.collapseIfReady()
            }
        }

        private func collapseIfReady() {
            guard ready, !collapsed, view.window != nil,
                  let navigation = navigationController,
                  let top = navigation.topViewController,
                  let search = top.navigationItem.searchController,
                  let scroll = Self.noteList(in: top.view) else { return }
            search.searchBar.searchTextField.accessibilityIdentifier = "library-search"
            top.navigationItem.hidesSearchBarWhenScrolling = true
            collapsed = true
            // Never undo a restored query, keyboard focus, or a user's gesture.
            guard queryIsEmpty, !search.isActive, !scroll.isTracking,
                  !scroll.isDragging, !scroll.isDecelerating else { return }
            navigation.view.layoutIfNeeded()
            search.searchBar.sizeToFit()
            let height = search.searchBar.bounds.height
            guard height > 0 else { return }
            scroll.setContentOffset(
                CGPoint(x: scroll.contentOffset.x, y: -scroll.adjustedContentInset.top + height),
                animated: false)
        }

        private static func noteList(in view: UIView) -> UIScrollView? {
            if let collection = view as? UICollectionView { return collection }
            if let table = view as? UITableView { return table }
            for child in view.subviews {
                if let list = noteList(in: child) { return list }
            }
            return nil
        }
    }
}
