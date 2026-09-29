from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="RotateApiTokenRequest")



@_attrs_define
class RotateApiTokenRequest:
    """ Request body for rotating a token's secret.

        Attributes:
            grace_seconds (int | None | Unset): How long the old secret keeps working, in seconds (default a day,
                at most a week; 0 retires it at once).
     """

    grace_seconds: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        grace_seconds: int | None | Unset
        if isinstance(self.grace_seconds, Unset):
            grace_seconds = UNSET
        else:
            grace_seconds = self.grace_seconds


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if grace_seconds is not UNSET:
            field_dict["grace_seconds"] = grace_seconds

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_grace_seconds(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        grace_seconds = _parse_grace_seconds(d.pop("grace_seconds", UNSET))


        rotate_api_token_request = cls(
            grace_seconds=grace_seconds,
        )


        rotate_api_token_request.additional_properties = d
        return rotate_api_token_request

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
