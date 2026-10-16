from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="MediaRecordingResponse")



@_attrs_define
class MediaRecordingResponse:
    """ What the node answered a recording request with.

        Attributes:
            call_id (str):
            status (str): `recording` or `stopped`.
            recording_id (None | str | Unset): The recording's id, when one started.
            path (None | str | Unset): The file it is written to, when one started.
            consent (None | str | Unset): What the parties were told: `announced`, `failed` or `not_required`.
     """

    call_id: str
    status: str
    recording_id: None | str | Unset = UNSET
    path: None | str | Unset = UNSET
    consent: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        call_id = self.call_id

        status = self.status

        recording_id: None | str | Unset
        if isinstance(self.recording_id, Unset):
            recording_id = UNSET
        else:
            recording_id = self.recording_id

        path: None | str | Unset
        if isinstance(self.path, Unset):
            path = UNSET
        else:
            path = self.path

        consent: None | str | Unset
        if isinstance(self.consent, Unset):
            consent = UNSET
        else:
            consent = self.consent


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "call_id": call_id,
            "status": status,
        })
        if recording_id is not UNSET:
            field_dict["recording_id"] = recording_id
        if path is not UNSET:
            field_dict["path"] = path
        if consent is not UNSET:
            field_dict["consent"] = consent

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        call_id = d.pop("call_id")

        status = d.pop("status")

        def _parse_recording_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        recording_id = _parse_recording_id(d.pop("recording_id", UNSET))


        def _parse_path(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        path = _parse_path(d.pop("path", UNSET))


        def _parse_consent(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        consent = _parse_consent(d.pop("consent", UNSET))


        media_recording_response = cls(
            call_id=call_id,
            status=status,
            recording_id=recording_id,
            path=path,
            consent=consent,
        )


        media_recording_response.additional_properties = d
        return media_recording_response

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
