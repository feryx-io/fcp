from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="PurgeReport")



@_attrs_define
class PurgeReport:
    """ What a purge did, counted.

        Attributes:
            devices_unassigned (int):
            lines_released (int):
            extensions_released (int):
            mailboxes_kept (int):
            calling_settings_removed (int):
            others_settings_changed (int):
            api_tokens_removed (int):
            hoteling_sessions_ended (int):
            queue_memberships_removed (int):
            declared_locations_removed (int):
            meetings_moved (int):
            meetings_unowned (int):
            registrations_removed (int):
            presence_removed (int):
            files_queued (int):
            removed_aors (list[str] | Unset): Every registration address removed, for the call managers to
                forget (`registration.unregistered`).
     """

    devices_unassigned: int
    lines_released: int
    extensions_released: int
    mailboxes_kept: int
    calling_settings_removed: int
    others_settings_changed: int
    api_tokens_removed: int
    hoteling_sessions_ended: int
    queue_memberships_removed: int
    declared_locations_removed: int
    meetings_moved: int
    meetings_unowned: int
    registrations_removed: int
    presence_removed: int
    files_queued: int
    removed_aors: list[str] | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        devices_unassigned = self.devices_unassigned

        lines_released = self.lines_released

        extensions_released = self.extensions_released

        mailboxes_kept = self.mailboxes_kept

        calling_settings_removed = self.calling_settings_removed

        others_settings_changed = self.others_settings_changed

        api_tokens_removed = self.api_tokens_removed

        hoteling_sessions_ended = self.hoteling_sessions_ended

        queue_memberships_removed = self.queue_memberships_removed

        declared_locations_removed = self.declared_locations_removed

        meetings_moved = self.meetings_moved

        meetings_unowned = self.meetings_unowned

        registrations_removed = self.registrations_removed

        presence_removed = self.presence_removed

        files_queued = self.files_queued

        removed_aors: list[str] | Unset = UNSET
        if not isinstance(self.removed_aors, Unset):
            removed_aors = self.removed_aors




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "devices_unassigned": devices_unassigned,
            "lines_released": lines_released,
            "extensions_released": extensions_released,
            "mailboxes_kept": mailboxes_kept,
            "calling_settings_removed": calling_settings_removed,
            "others_settings_changed": others_settings_changed,
            "api_tokens_removed": api_tokens_removed,
            "hoteling_sessions_ended": hoteling_sessions_ended,
            "queue_memberships_removed": queue_memberships_removed,
            "declared_locations_removed": declared_locations_removed,
            "meetings_moved": meetings_moved,
            "meetings_unowned": meetings_unowned,
            "registrations_removed": registrations_removed,
            "presence_removed": presence_removed,
            "files_queued": files_queued,
        })
        if removed_aors is not UNSET:
            field_dict["removed_aors"] = removed_aors

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        devices_unassigned = d.pop("devices_unassigned")

        lines_released = d.pop("lines_released")

        extensions_released = d.pop("extensions_released")

        mailboxes_kept = d.pop("mailboxes_kept")

        calling_settings_removed = d.pop("calling_settings_removed")

        others_settings_changed = d.pop("others_settings_changed")

        api_tokens_removed = d.pop("api_tokens_removed")

        hoteling_sessions_ended = d.pop("hoteling_sessions_ended")

        queue_memberships_removed = d.pop("queue_memberships_removed")

        declared_locations_removed = d.pop("declared_locations_removed")

        meetings_moved = d.pop("meetings_moved")

        meetings_unowned = d.pop("meetings_unowned")

        registrations_removed = d.pop("registrations_removed")

        presence_removed = d.pop("presence_removed")

        files_queued = d.pop("files_queued")

        removed_aors = cast(list[str], d.pop("removed_aors", UNSET))


        purge_report = cls(
            devices_unassigned=devices_unassigned,
            lines_released=lines_released,
            extensions_released=extensions_released,
            mailboxes_kept=mailboxes_kept,
            calling_settings_removed=calling_settings_removed,
            others_settings_changed=others_settings_changed,
            api_tokens_removed=api_tokens_removed,
            hoteling_sessions_ended=hoteling_sessions_ended,
            queue_memberships_removed=queue_memberships_removed,
            declared_locations_removed=declared_locations_removed,
            meetings_moved=meetings_moved,
            meetings_unowned=meetings_unowned,
            registrations_removed=registrations_removed,
            presence_removed=presence_removed,
            files_queued=files_queued,
            removed_aors=removed_aors,
        )


        purge_report.additional_properties = d
        return purge_report

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
