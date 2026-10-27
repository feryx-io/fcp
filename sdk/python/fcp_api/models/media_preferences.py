from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="MediaPreferences")



@_attrs_define
class MediaPreferences:
    """ Media preferences for a user account.
    These settings control default behavior for calls to/from this user.

        Attributes:
            force_media_anchoring (bool | Unset): Force media to always flow through server (for testing/debugging)
            enable_dtmf_detection (bool | Unset): Enable DTMF detection for this user's calls
            record_all_calls (bool | Unset): Record all calls for this user
            record_on_demand (bool | Unset): The user may record a call they are on, and stop that recording,
                from their app (`docs/RECORD_AND_CONTACTS.md` §2). Set by an
                administrator only.
            preferred_codecs (list[str] | Unset): Preferred audio codecs in priority order
     """

    force_media_anchoring: bool | Unset = UNSET
    enable_dtmf_detection: bool | Unset = UNSET
    record_all_calls: bool | Unset = UNSET
    record_on_demand: bool | Unset = UNSET
    preferred_codecs: list[str] | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        force_media_anchoring = self.force_media_anchoring

        enable_dtmf_detection = self.enable_dtmf_detection

        record_all_calls = self.record_all_calls

        record_on_demand = self.record_on_demand

        preferred_codecs: list[str] | Unset = UNSET
        if not isinstance(self.preferred_codecs, Unset):
            preferred_codecs = self.preferred_codecs




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if force_media_anchoring is not UNSET:
            field_dict["force_media_anchoring"] = force_media_anchoring
        if enable_dtmf_detection is not UNSET:
            field_dict["enable_dtmf_detection"] = enable_dtmf_detection
        if record_all_calls is not UNSET:
            field_dict["record_all_calls"] = record_all_calls
        if record_on_demand is not UNSET:
            field_dict["record_on_demand"] = record_on_demand
        if preferred_codecs is not UNSET:
            field_dict["preferred_codecs"] = preferred_codecs

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        force_media_anchoring = d.pop("force_media_anchoring", UNSET)

        enable_dtmf_detection = d.pop("enable_dtmf_detection", UNSET)

        record_all_calls = d.pop("record_all_calls", UNSET)

        record_on_demand = d.pop("record_on_demand", UNSET)

        preferred_codecs = cast(list[str], d.pop("preferred_codecs", UNSET))


        media_preferences = cls(
            force_media_anchoring=force_media_anchoring,
            enable_dtmf_detection=enable_dtmf_detection,
            record_all_calls=record_all_calls,
            record_on_demand=record_on_demand,
            preferred_codecs=preferred_codecs,
        )


        media_preferences.additional_properties = d
        return media_preferences

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
