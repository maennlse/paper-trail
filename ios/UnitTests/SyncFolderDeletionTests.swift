// SPDX-License-Identifier: GPL-3.0-or-later
import XCTest

final class SyncFolderDeletionTests: XCTestCase {
    func testOnlyPublishedLiveIDsCanBeConfirmed() throws {
        XCTAssertNil(SyncFolderDeletion(id: nil, name: "Notes", published: true, deleted: false))
        XCTAssertNil(SyncFolderDeletion(id: "", name: "Notes", published: true, deleted: false))
        XCTAssertNil(SyncFolderDeletion(id: "draft", name: "Notes", published: false, deleted: false))
        XCTAssertNil(SyncFolderDeletion(id: "erased", name: "Notes", published: true, deleted: true))
        let first = try XCTUnwrap(SyncFolderDeletion(id: "first", name: "Notes", published: true, deleted: false))
        let second = try XCTUnwrap(SyncFolderDeletion(id: "second", name: "Notes", published: true, deleted: false))
        XCTAssertNotEqual(first.id, second.id)
        XCTAssertTrue(first.message.contains("Folder ID: first"))
        XCTAssertTrue(first.message.contains("unsaved edits"))
        XCTAssertTrue(first.message.contains("Server backups"))
    }

    func testDeletionRequiresSignInUnlockAndNoOtherOperation() {
        for signedIn in [false, true] {
            for unlocked in [false, true] {
                for busy in [false, true] {
                    for recovery in [false, true] {
                        XCTAssertEqual(
                            SyncFolderDeletion.isAvailable(signedIn: signedIn, unlocked: unlocked,
                                                           busy: busy, hasRecovery: recovery),
                            signedIn && unlocked && !busy && !recovery
                        )
                    }
                }
            }
        }
    }
}
