from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.find_me_step import FindMeStep





T = TypeVar("T", bound="FindMe")



@_attrs_define
class FindMe:
    """ A user's find-me: steps rung one after another once the user's own
    phones have rung, before the no-answer treatment.

        Attributes:
            enabled (bool | Unset):
            steps (list[FindMeStep] | Unset):
     """

    enabled: bool | Unset = UNSET
    steps: list[FindMeStep] | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.find_me_step import FindMeStep # noqa: PLC0415
        enabled = self.enabled

        steps: list[dict[str, Any]] | Unset = UNSET
        if not isinstance(self.steps, Unset):
            steps = []
            for steps_item_data in self.steps:
                steps_item = steps_item_data.to_dict()
                steps.append(steps_item)




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if enabled is not UNSET:
            field_dict["enabled"] = enabled
        if steps is not UNSET:
            field_dict["steps"] = steps

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.find_me_step import FindMeStep # noqa: PLC0415
        d = dict(src_dict)
        enabled = d.pop("enabled", UNSET)

        _steps = d.pop("steps", UNSET)
        steps: list[FindMeStep] | Unset = UNSET
        if _steps is not UNSET:
            steps = []
            for steps_item_data in _steps:
                steps_item = FindMeStep.from_dict(steps_item_data)



                steps.append(steps_item)


        find_me = cls(
            enabled=enabled,
            steps=steps,
        )


        find_me.additional_properties = d
        return find_me

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
