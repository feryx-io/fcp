from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="MyCallQuality")



@_attrs_define
class MyCallQuality:
    """ How one of the user's calls went, by their phone: on `/me/calls`.

        Attributes:
            mos (float):
            poor (bool): Either score under 3.1.
            far_mos (float | None | Unset):
     """

    mos: float
    poor: bool
    far_mos: float | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        mos = self.mos

        poor = self.poor

        far_mos: float | None | Unset
        if isinstance(self.far_mos, Unset):
            far_mos = UNSET
        else:
            far_mos = self.far_mos


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "mos": mos,
            "poor": poor,
        })
        if far_mos is not UNSET:
            field_dict["far_mos"] = far_mos

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        mos = d.pop("mos")

        poor = d.pop("poor")

        def _parse_far_mos(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        far_mos = _parse_far_mos(d.pop("far_mos", UNSET))


        my_call_quality = cls(
            mos=mos,
            poor=poor,
            far_mos=far_mos,
        )


        my_call_quality.additional_properties = d
        return my_call_quality

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
