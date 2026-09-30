from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="RotateWebhookSecretRequest")



@_attrs_define
class RotateWebhookSecretRequest:
    """ A rotation.

        Attributes:
            overlap_seconds (int | None | Unset): How long the old secret keeps signing beside the new one, in
                seconds (default a day, at most a week; 0 retires it at once).
     """

    overlap_seconds: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        overlap_seconds: int | None | Unset
        if isinstance(self.overlap_seconds, Unset):
            overlap_seconds = UNSET
        else:
            overlap_seconds = self.overlap_seconds


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if overlap_seconds is not UNSET:
            field_dict["overlap_seconds"] = overlap_seconds

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_overlap_seconds(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        overlap_seconds = _parse_overlap_seconds(d.pop("overlap_seconds", UNSET))


        rotate_webhook_secret_request = cls(
            overlap_seconds=overlap_seconds,
        )


        rotate_webhook_secret_request.additional_properties = d
        return rotate_webhook_secret_request

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
