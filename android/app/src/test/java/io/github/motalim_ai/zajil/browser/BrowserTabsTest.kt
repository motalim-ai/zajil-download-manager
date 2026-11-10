package io.github.motalim_ai.zajil.browser

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class BrowserTabsTest {
    private val tabs = listOf(
        TabInfo(1, "Regular 1", "https://example.com/1", privateMode = false),
        TabInfo(2, "Private 1", "https://example.com/2", privateMode = true),
        TabInfo(3, "Regular 2", "https://example.com/3", privateMode = false),
    )

    @Test
    fun groupsKeepRegularAndPrivateTabsSeparate() {
        assertEquals(listOf(1L, 3L), BrowserTabs.inGroup(tabs, privateMode = false).map { it.id })
        assertEquals(listOf(2L), BrowserTabs.inGroup(tabs, privateMode = true).map { it.id })
    }

    @Test
    fun switchingGroupsSelectsItsLastTabWithoutClosingTheOtherGroup() {
        assertEquals(2L, BrowserTabs.lastInGroup(tabs, privateMode = true)?.id)
        assertEquals(3L, BrowserTabs.lastInGroup(tabs, privateMode = false)?.id)
        assertEquals(3, tabs.size)
    }

    @Test
    fun emptyGroupDoesNotSelectATabFromTheOtherGroup() {
        assertNull(BrowserTabs.lastInGroup(tabs.filterNot { it.privateMode }, privateMode = true))
    }

    @Test
    fun closingTheLastPrivateTabRequiresConfirmationWithoutAffectingRegularTabs() {
        assertEquals(true, BrowserTabs.needsPrivateConfirmation(tabs, id = 2))
        assertEquals(false, BrowserTabs.needsPrivateConfirmation(tabs, id = 3))
        assertEquals(
            false,
            BrowserTabs.needsPrivateConfirmation(
                tabs + TabInfo(4, "Private 2", "https://example.com/4", privateMode = true),
                id = 2,
            ),
        )
    }
}
